import { Buffer } from "node:buffer";
import type { PlanetVideoPacket } from "../vendor/linejs-call/entry.ts";

export type VideoTransport = {
  sendVideo(
    payload: Uint8Array,
    opts: { timestamp: number; marker: boolean },
  ): Promise<void>;
  receiveVideo(): AsyncIterable<PlanetVideoPacket>;
};

export type VideoControls = {
  cameraEnabled: boolean;
  mirror: boolean;
};

export type CallVideoOptions = {
  transport: VideoTransport;
  signal: AbortSignal;
  controls: VideoControls;
  videoDevice?: string;
  onFrame: (side: "local" | "remote", jpeg: Uint8Array | null) => void;
  onEnded?: (error?: Error) => void;
};

const VIDEO_FPS = 15;
const RTP_CLOCK_HZ = 90_000;
const RTP_TS_STEP = Math.round(RTP_CLOCK_HZ / VIDEO_FPS); // 6000 ticks/frame
const MAX_RTP_PAYLOAD = 1150;
const ANNEX_B_START = new Uint8Array([0x00, 0x00, 0x00, 0x01]);

/** Stateful H.264 Annex-B byte-stream splitter returning raw NAL units (without start codes). */
export class AnnexBNaluSplitter {
  #buf: Uint8Array = new Uint8Array(0);

  push(chunk: Uint8Array): Uint8Array[] {
    if (!chunk.length) return [];
    const merged = new Uint8Array(this.#buf.length + chunk.length);
    merged.set(this.#buf, 0);
    merged.set(chunk, this.#buf.length);
    const starts: Array<{ pos: number; len: number }> = [];
    for (let i = 0; i + 2 < merged.length; i++) {
      if (merged[i] === 0 && merged[i + 1] === 0) {
        if (merged[i + 2] === 1) {
          starts.push({ pos: i, len: 3 });
          i += 2;
        } else if (
          i + 3 < merged.length && merged[i + 2] === 0 && merged[i + 3] === 1
        ) {
          starts.push({ pos: i, len: 4 });
          i += 3;
        }
      }
    }
    if (starts.length === 0) {
      // Keep last 3 bytes in case a start code straddles the chunk boundary.
      this.#buf = merged.length > 3 ? merged.slice(merged.length - 3) : merged;
      return [];
    }
    const nalus: Uint8Array[] = [];
    for (let i = 0; i + 1 < starts.length; i++) {
      const start = starts[i].pos + starts[i].len;
      const end = starts[i + 1].pos;
      if (end > start) {
        nalus.push(merged.slice(start, end));
      }
    }
    this.#buf = merged.slice(starts[starts.length - 1].pos);
    return nalus;
  }

  flush(): Uint8Array[] {
    if (this.#buf.length < 4) {
      this.#buf = new Uint8Array(0);
      return [];
    }
    let offset = 0;
    if (
      this.#buf[0] === 0 && this.#buf[1] === 0 && this.#buf[2] === 0 &&
      this.#buf[3] === 1
    ) {
      offset = 4;
    } else if (this.#buf[0] === 0 && this.#buf[1] === 0 && this.#buf[2] === 1) {
      offset = 3;
    }
    const out = offset > 0 && this.#buf.length > offset
      ? [this.#buf.slice(offset)]
      : [];
    this.#buf = new Uint8Array(0);
    return out;
  }
}

/** RFC 6184 H.264 RTP packetizer: Single NAL unit or FU-A fragmentation. */
export function packetizeH264Nalu(
  nalu: Uint8Array,
  maxPayload = MAX_RTP_PAYLOAD,
): Array<{ payload: Uint8Array; isLastFragment: boolean }> {
  if (!nalu.length) return [];
  if (nalu.length <= maxPayload) {
    return [{ payload: nalu.slice(), isLastFragment: true }];
  }
  const nalHeader = nalu[0];
  const fNri = nalHeader & 0xe0;
  const nalType = nalHeader & 0x1f;
  const fuIndicator = fNri | 28; // FU-A
  const maxFragData = Math.max(16, maxPayload - 2);
  const out: Array<{ payload: Uint8Array; isLastFragment: boolean }> = [];
  let offset = 1;
  while (offset < nalu.length) {
    const end = Math.min(nalu.length, offset + maxFragData);
    const isStart = offset === 1;
    const isEnd = end === nalu.length;
    const fuHeader = (isStart ? 0x80 : 0) | (isEnd ? 0x40 : 0) | nalType;
    const pkt = new Uint8Array(2 + (end - offset));
    pkt[0] = fuIndicator;
    pkt[1] = fuHeader;
    pkt.set(nalu.subarray(offset, end), 2);
    out.push({ payload: pkt, isLastFragment: isEnd });
    offset = end;
  }
  return out;
}

function isLikelyH264RtpStart(bytes: Uint8Array, offset: number): boolean {
  if (offset >= bytes.length) return false;
  const b0 = bytes[offset];
  if ((b0 & 0x80) !== 0) return false; // forbidden_zero_bit must be 0
  const nalType = b0 & 0x1f;
  if (nalType >= 1 && nalType <= 23) return true;
  if (nalType === 24) {
    if (offset + 3 >= bytes.length) return false;
    const sz = (bytes[offset + 1] << 8) | bytes[offset + 2];
    return sz > 0 && offset + 3 + sz <= bytes.length;
  }
  if (nalType === 28) {
    if (offset + 2 >= bytes.length) return false;
    const innerType = bytes[offset + 1] & 0x1f;
    return innerType >= 1 && innerType <= 23;
  }
  return false;
}

/** RFC 6184 H.264 RTP depacketizer emitting Annex-B framed NAL units. */
export class H264RtpDepacketizer {
  #fuChunks: Uint8Array[] = [];
  #fuBytes = 0;
  #fuActive = false;

  push(rawPayload: Uint8Array): Uint8Array[] {
    if (!rawPayload.length) return [];
    let payload = rawPayload;
    // Strip a 1-byte PLANET prefix if rawPayload[0] is not a valid H.264 RTP header
    // (e.g. forbidden_zero_bit set or nalType === 0) while rawPayload[1] is.
    if (
      !isLikelyH264RtpStart(rawPayload, 0) &&
      rawPayload.length > 2 &&
      isLikelyH264RtpStart(rawPayload, 1)
    ) {
      payload = rawPayload.subarray(1);
    }
    const header = payload[0];
    if ((header & 0x80) !== 0) return [];
    const nalType = header & 0x1f;

    if (nalType >= 1 && nalType <= 23) {
      this.#resetFu();
      return [this.#withAnnexB(payload)];
    }

    if (nalType === 24) {
      // STAP-A: multiple NALUs prefixed by 16-bit big-endian lengths
      this.#resetFu();
      const out: Uint8Array[] = [];
      let off = 1;
      while (off + 2 <= payload.length) {
        const size = (payload[off] << 8) | payload[off + 1];
        off += 2;
        if (size <= 0 || off + size > payload.length) break;
        out.push(this.#withAnnexB(payload.subarray(off, off + size)));
        off += size;
      }
      return out;
    }

    if (nalType === 28 && payload.length >= 2) {
      // FU-A fragmentation unit
      const fuHeader = payload[1];
      const start = (fuHeader & 0x80) !== 0;
      const end = (fuHeader & 0x40) !== 0;
      const origType = fuHeader & 0x1f;
      if (start) {
        this.#resetFu();
        const reconstructedHeader = new Uint8Array([
          (header & 0xe0) | origType,
        ]);
        const frag = payload.subarray(2);
        this.#fuChunks = [reconstructedHeader, frag.slice()];
        this.#fuBytes = 1 + frag.length;
        this.#fuActive = true;
      } else if (this.#fuActive) {
        const frag = payload.subarray(2);
        this.#fuChunks.push(frag.slice());
        this.#fuBytes += frag.length;
        if (this.#fuBytes > 1024 * 1024) {
          this.#resetFu();
          return [];
        }
      } else {
        return [];
      }
      if (end && this.#fuActive) {
        const assembled = new Uint8Array(ANNEX_B_START.length + this.#fuBytes);
        assembled.set(ANNEX_B_START, 0);
        let off = ANNEX_B_START.length;
        for (const c of this.#fuChunks) {
          assembled.set(c, off);
          off += c.length;
        }
        this.#resetFu();
        return [assembled];
      }
      return [];
    }

    return [];
  }

  #resetFu() {
    this.#fuChunks = [];
    this.#fuBytes = 0;
    this.#fuActive = false;
  }

  #withAnnexB(nalu: Uint8Array): Uint8Array {
    const out = new Uint8Array(ANNEX_B_START.length + nalu.length);
    out.set(ANNEX_B_START, 0);
    out.set(nalu, ANNEX_B_START.length);
    return out;
  }
}

/** Stateful MJPEG image2pipe splitter extracting complete SOI (FF D8) .. EOI (FF D9) frames. */
export class MjpegFrameSplitter {
  #buf: Uint8Array = new Uint8Array(0);

  push(chunk: Uint8Array): Uint8Array[] {
    if (!chunk.length) return [];
    const merged = new Uint8Array(this.#buf.length + chunk.length);
    merged.set(this.#buf, 0);
    merged.set(chunk, this.#buf.length);
    const frames: Uint8Array[] = [];
    let searchFrom = 0;
    while (searchFrom + 3 < merged.length) {
      let soi = -1;
      for (let i = searchFrom; i + 1 < merged.length; i++) {
        if (merged[i] === 0xff && merged[i + 1] === 0xd8) {
          soi = i;
          break;
        }
      }
      if (soi < 0) {
        this.#buf = merged.slice(merged.length - 1);
        return frames;
      }
      let eoi = -1;
      for (let i = soi + 2; i + 1 < merged.length; i++) {
        if (merged[i] === 0xff && merged[i + 1] === 0xd9) {
          eoi = i + 2;
          break;
        }
      }
      if (eoi < 0) {
        const tail = merged.slice(soi);
        // Bound buffer if corrupted stream lacks EOI
        this.#buf = tail.length > 512 * 1024 ? new Uint8Array(0) : tail;
        return frames;
      }
      frames.push(merged.slice(soi, eoi));
      searchFrom = eoi;
    }
    this.#buf = searchFrom < merged.length
      ? merged.slice(searchFrom)
      : new Uint8Array(0);
    return frames;
  }
}

function bytesToHex(bytes: Uint8Array, maxBytes: number): string {
  const view = bytes.subarray(0, Math.min(bytes.length, maxBytes));
  return Array.from(view, (b) => b.toString(16).padStart(2, "0")).join("");
}

function isVclNalu(nalu: Uint8Array): boolean {
  if (!nalu.length) return false;
  const t = nalu[0] & 0x1f;
  return t >= 1 && t <= 5;
}

function defaultCameraDevice(configured?: string): string {
  const trimmed = configured?.trim();
  if (trimmed && trimmed !== "default") return trimmed;
  return "/dev/video0";
}

function spawnCameraEncoder(device: string, explicitSize: boolean) {
  const args = [
    "-hide_banner",
    "-loglevel",
    "error",
    "-f",
    "v4l2",
    "-framerate",
    String(VIDEO_FPS),
    ...(explicitSize ? ["-video_size", "640x360"] : []),
    "-i",
    device,
    "-vf",
    "scale=640:360:force_original_aspect_ratio=decrease,pad=640:360:(ow-iw)/2:(oh-ih)/2",
    "-pix_fmt",
    "yuv420p",
    "-c:v",
    "libx264",
    "-profile:v",
    "baseline",
    "-level",
    "3.1",
    "-preset",
    "ultrafast",
    "-tune",
    "zerolatency",
    "-b:v",
    "600k",
    "-maxrate",
    "800k",
    "-bufsize",
    "800k",
    "-g",
    String(VIDEO_FPS * 2),
    "-keyint_min",
    String(VIDEO_FPS),
    "-x264-params",
    "repeat-headers=1:slice-max-size=1100",
    "-f",
    "h264",
    "pipe:1",
  ];
  return new Deno.Command("ffmpeg", {
    args,
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
}

function spawnH264ToMjpegDecoder(
  opts: { width: number; mirror: boolean; q: number },
) {
  const vf = opts.mirror
    ? `scale=${opts.width}:-2,hflip`
    : `scale=${opts.width}:-2`;
  return new Deno.Command("ffmpeg", {
    args: [
      "-hide_banner",
      "-loglevel",
      "error",
      "-fflags",
      "nobuffer",
      "-flags",
      "low_delay",
      "-f",
      "h264",
      "-i",
      "pipe:0",
      "-vf",
      vf,
      "-c:v",
      "mjpeg",
      "-q:v",
      String(opts.q),
      "-f",
      "image2pipe",
      "pipe:1",
    ],
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
}

function killChild(child: Deno.ChildProcess | undefined) {
  if (!child) return;
  try {
    child.kill("SIGTERM");
  } catch { /* already exited */ }
}

export function startCallVideoIO(options: CallVideoOptions): {
  stop(): Promise<void>;
  done: Promise<void>;
} {
  const cancellation = new AbortController();
  const startedAt = performance.now();
  const stats = {
    txVideoFrames: 0,
    txVideoPackets: 0,
    txVideoBytes: 0,
    localPreviewFrames: 0,
    rxVideoPackets: 0,
    rxVideoNalus: 0,
    rxVideoFrames: 0,
    rxFirst5Headers: [] as string[],
  };
  let closing: Promise<void> | undefined;
  let txEncChild: Deno.ChildProcess | undefined;
  let localDecChild: Deno.ChildProcess | undefined;
  let remoteDecChild: Deno.ChildProcess | undefined;

  function reportVideo(event: "stopped" | "interval") {
    console.error(
      "[call video stats]",
      JSON.stringify({
        event,
        elapsedMs: Math.round(performance.now() - startedAt),
        ...stats,
        cameraEnabled: options.controls.cameraEnabled,
      }),
    );
  }

  function pause(ms: number) {
    return new Promise<void>((resolve) => {
      const finish = () => {
        clearTimeout(timer);
        cancellation.signal.removeEventListener("abort", finish);
        resolve();
      };
      const timer = setTimeout(finish, ms);
      cancellation.signal.addEventListener("abort", finish, { once: true });
    });
  }

  async function stop() {
    if (closing) return await closing;
    cancellation.abort();
    options.signal.removeEventListener("abort", parentAbort);
    killChild(txEncChild);
    killChild(localDecChild);
    killChild(remoteDecChild);
    closing = Promise.allSettled([
      txEncChild?.status,
      localDecChild?.status,
      remoteDecChild?.status,
    ]).then(() => {
      options.onFrame("local", null);
      options.onFrame("remote", null);
      reportVideo("stopped");
    });
    await closing;
  }

  function parentAbort() {
    void stop();
  }
  options.signal.addEventListener("abort", parentAbort, { once: true });
  if (options.signal.aborted) parentAbort();

  async function runCaptureCycle(
    device: string,
    explicitSize: boolean,
  ): Promise<boolean> {
    const enc = spawnCameraEncoder(device, explicitSize);
    txEncChild = enc;
    const dec = spawnH264ToMjpegDecoder({
      width: 240,
      mirror: options.controls.mirror,
      q: 7,
    });
    localDecChild = dec;

    const encReader = enc.stdout.getReader();
    const decWriter = dec.stdin.getWriter();
    const decReader = dec.stdout.getReader();
    const splitter = new AnnexBNaluSplitter();
    const mjpeg = new MjpegFrameSplitter();
    let gotAnyBytes = false;
    let rtpTs = 0;
    let lastPreviewAt = 0;

    // Drain stderr silently (log sanitized error summary only)
    const encErr = new Response(enc.stderr).text().catch(() => "");
    const decErr = new Response(dec.stderr).text().catch(() => "");

    const localPreviewLoop = (async () => {
      try {
        while (!cancellation.signal.aborted && options.controls.cameraEnabled) {
          const { value, done } = await decReader.read();
          if (done || !value) break;
          const frames = mjpeg.push(value);
          const now = performance.now();
          if (frames.length > 0 && now - lastPreviewAt >= 80) {
            lastPreviewAt = now;
            stats.localPreviewFrames++;
            options.onFrame("local", frames[frames.length - 1]);
          }
        }
      } catch { /* stopped */ }
    })();

    const watchDisable = (async () => {
      while (!cancellation.signal.aborted && options.controls.cameraEnabled) {
        await pause(120);
      }
      killChild(enc);
      killChild(dec);
    })();

    try {
      while (!cancellation.signal.aborted && options.controls.cameraEnabled) {
        const { value, done } = await encReader.read();
        if (done || !value) break;
        gotAnyBytes = true;
        // Mirror H.264 stream into lightweight local preview decoder
        void decWriter.write(value).catch(() => {});
        const nalus = splitter.push(value);
        for (const nalu of nalus) {
          if (cancellation.signal.aborted || !options.controls.cameraEnabled) {
            break;
          }
          const vcl = isVclNalu(nalu);
          const frags = packetizeH264Nalu(nalu, MAX_RTP_PAYLOAD);
          for (const frag of frags) {
            await options.transport.sendVideo(frag.payload, {
              timestamp: rtpTs,
              marker: vcl && frag.isLastFragment,
            });
            stats.txVideoPackets++;
            stats.txVideoBytes += frag.payload.length;
          }
          if (vcl) {
            stats.txVideoFrames++;
            rtpTs = (rtpTs + RTP_TS_STEP) >>> 0;
          }
        }
      }
    } catch {
      // Transport ended or camera stopped
    } finally {
      killChild(enc);
      killChild(dec);
      void encReader.cancel().catch(() => {});
      void decReader.cancel().catch(() => {});
      void decWriter.close().catch(() => {});
      await Promise.allSettled([
        localPreviewLoop,
        watchDisable,
        enc.status,
        dec.status,
      ]);
      const errText = (await encErr).trim();
      void decErr;
      if (errText && !cancellation.signal.aborted && !gotAnyBytes) {
        console.error(
          "[call video] camera capture error:",
          errText.slice(0, 160),
        );
      }
      if (txEncChild === enc) txEncChild = undefined;
      if (localDecChild === dec) localDecChild = undefined;
    }
    return gotAnyBytes;
  }

  async function txLoop() {
    const device = defaultCameraDevice(options.videoDevice);
    let useExplicitSize = true;
    while (!cancellation.signal.aborted) {
      if (!options.controls.cameraEnabled) {
        options.onFrame("local", null);
        await pause(150);
        continue;
      }
      const ok = await runCaptureCycle(device, useExplicitSize);
      if (cancellation.signal.aborted) break;
      if (!options.controls.cameraEnabled) {
        options.onFrame("local", null);
        continue;
      }
      if (!ok && useExplicitSize) {
        // Retry without explicit -video_size so V4L2 picks native mode
        useExplicitSize = false;
        await pause(100);
        continue;
      }
      // Camera disconnected or busy — retry softly without crashing audio call
      options.onFrame("local", null);
      await pause(1500);
    }
  }

  async function rxLoop() {
    const depacketizer = new H264RtpDepacketizer();
    const mjpeg = new MjpegFrameSplitter();
    let dec: Deno.ChildProcess | undefined;
    let decWriter: WritableStreamDefaultWriter<Uint8Array> | undefined;
    let decReaderLoop: Promise<void> | undefined;
    let lastRemoteAt = 0;

    function ensureDecoder() {
      if (dec) return;
      dec = spawnH264ToMjpegDecoder({ width: 640, mirror: false, q: 5 });
      remoteDecChild = dec;
      decWriter = dec.stdin.getWriter();
      const reader = dec.stdout.getReader();
      void new Response(dec.stderr).text().catch(() => {});
      decReaderLoop = (async () => {
        try {
          while (!cancellation.signal.aborted) {
            const { value, done } = await reader.read();
            if (done || !value) break;
            const frames = mjpeg.push(value);
            const now = performance.now();
            if (frames.length > 0 && now - lastRemoteAt >= 45) {
              lastRemoteAt = now;
              stats.rxVideoFrames++;
              options.onFrame("remote", frames[frames.length - 1]);
            }
          }
        } catch { /* stopped */ }
      })();
    }

    try {
      for await (const pkt of options.transport.receiveVideo()) {
        if (cancellation.signal.aborted) break;
        stats.rxVideoPackets++;
        if (stats.rxFirst5Headers.length < 5) {
          stats.rxFirst5Headers.push(
            `${pkt.ssrc}|${pkt.marker ? 1 : 0}|${pkt.payload.length}|${
              bytesToHex(pkt.payload, 6)
            }`,
          );
        }
        const nalus = depacketizer.push(pkt.payload);
        if (nalus.length > 0) {
          ensureDecoder();
          for (const nalu of nalus) {
            stats.rxVideoNalus++;
            await decWriter!.write(nalu).catch(() => {});
          }
        }
      }
    } catch {
      // Transport closed
    } finally {
      if (decWriter) void decWriter.close().catch(() => {});
      killChild(dec);
      await Promise.allSettled([decReaderLoop, dec?.status]);
      if (remoteDecChild === dec) remoteDecChild = undefined;
    }
  }

  const done = Promise.allSettled([txLoop(), rxLoop()])
    .then(() => closing)
    .then(() => {});
  return { stop, done };
}

export function encodeJpegBase64(jpeg: Uint8Array): string {
  return Buffer.from(jpeg).toString("base64");
}
