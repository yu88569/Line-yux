import {
  type CodecFactory,
  packetizeNativeGroupOpusPairs,
} from "../vendor/linejs-call/audio.ts";

type AudioTransport = {
  send(payload: Uint8Array, opts: { timestampStep: number }): Promise<void>;
  receive(): AsyncIterable<Uint8Array>;
};

export type AudioDevices = {
  input: ReadableStream<Uint8Array>;
  output: WritableStream<Uint8Array>;
  close(): Promise<void>;
};

type AudioControls = {
  muted: boolean;
  deafened: boolean;
  micGain: number;
  spkGain: number;
};

type AudioOptions = {
  transport: AudioTransport;
  codecFactory(): Promise<CodecFactory>;
  signal: AbortSignal;
  controls: AudioControls;
  inputDevice?: string;
  outputDevice?: string;
  opusSignal?: "auto" | "voice" | "music";
  pairedOpus?: boolean;
  spawnDevices?: () => AudioDevices;
  onEnded?: (error?: Error) => void;
};

const sampleRate = 48_000;
const frameSamples = 960;
const frameBytes = frameSamples * 2;
const maxQueueSamples = frameSamples * 10; // 200 ms
const maxSegmentAgeMs = 200;

type QueuedAudioSegment = {
  samples: Int16Array;
  offset: number;
  receivedAt: number;
};

function spawnAudioDevices(input: string, output: string): AudioDevices {
  const children: Deno.ChildProcess[] = [];
  const drains: Promise<unknown>[] = [];
  function spawn(args: string[], capture: boolean) {
    const child = new Deno.Command("ffmpeg", {
      args: ["-hide_banner", "-loglevel", "error", ...args],
      stdin: capture ? "null" : "piped",
      stdout: capture ? "piped" : "null",
      stderr: "piped",
    }).spawn();
    children.push(child);
    drains.push(
      child.stderr.pipeTo(
        new WritableStream({
          write(bytes) {
            console.error(
              "[call audio]",
              new TextDecoder().decode(bytes).trim(),
            );
          },
        }),
      ).catch(() => {}),
      child.status,
    );
    return child;
  }
  function close() {
    for (const child of children) {
      try {
        child.kill("SIGTERM");
      } catch { /* already exited */ }
    }
    return Promise.allSettled(drains).then(() => {});
  }
  try {
    const mic = spawn([
      "-f",
      "pulse",
      "-i",
      input,
      "-ac",
      "1",
      "-ar",
      String(sampleRate),
      "-f",
      "s16le",
      "pipe:1",
    ], true);
    const speaker = spawn([
      "-f",
      "s16le",
      "-ar",
      String(sampleRate),
      "-ac",
      "1",
      "-i",
      "pipe:0",
      "-f",
      "pulse",
      ...(output === "default" ? [] : ["-device", output]),
      "LINE GTK call",
    ], false);
    return { input: mic.stdout, output: speaker.stdin, close };
  } catch (error) {
    void close();
    throw error;
  }
}

function gain(samples: Int16Array, multiplier: number): Int16Array {
  if (multiplier === 1) return samples;
  return Int16Array.from(
    samples,
    (value) =>
      Math.max(-32768, Math.min(32767, Math.round(value * multiplier))),
  );
}

function signalLevel(samples: Int16Array) {
  let peak = 0;
  let squareSum = 0;
  for (const sample of samples) {
    peak = Math.max(peak, Math.abs(sample));
    squareSum += sample * sample;
  }
  const rmsDbfs = squareSum > 0
    ? Math.max(
      -120,
      Math.round(
        200 * Math.log10(Math.sqrt(squareSum / samples.length) / 32768),
      ) / 10,
    )
    : -120;
  return { peak, rmsDbfs };
}

function opusTocFrameSamples48k(toc: number): number {
  const cfg = (toc >>> 3) & 0x1f;
  if (cfg < 12) {
    return [480, 960, 1920, 2880][cfg & 0x03];
  }
  if (cfg < 16) {
    return (cfg & 0x01) === 0 ? 480 : 960;
  }
  return [120, 240, 480, 960][cfg & 0x03];
}

function isPairedVoipOpusAt(payload: Uint8Array, offset: number): boolean {
  if (offset + 3 > payload.length) return false;
  const toc = payload[offset];
  if ((toc & 0x04) !== 0 || (toc & 0x03) !== 3) return false;
  if (opusTocFrameSamples48k(toc) !== 960) return false;
  const countByte = payload[offset + 1];
  const remaining = payload.length - (offset + 2);
  if (countByte === 0x02) {
    return remaining >= 2 && remaining % 2 === 0;
  }
  if (countByte !== 0x82 || remaining < 2) return false;
  const b0 = payload[offset + 2];
  let lenBytes = 1;
  let leftLen = b0;
  if (b0 >= 252) {
    if (remaining < 3) return false;
    lenBytes = 2;
    leftLen = (b0 - 252) + 4 * payload[offset + 3];
  }
  const rightLen = remaining - lenBytes - leftLen;
  return leftLen >= 1 && rightLen >= 1;
}

function isSingleVoipOpusAt(payload: Uint8Array, offset: number): boolean {
  if (offset >= payload.length) return false;
  const toc = payload[offset];
  if ((toc & 0x04) !== 0 || (toc & 0x03) !== 0) return false;
  return opusTocFrameSamples48k(toc) >= 960;
}

export function extractPlanetOpusPacket(payload: Uint8Array): {
  packet: Uint8Array;
  prefixOffset: number;
} {
  if (payload.length <= 1) return { packet: payload, prefixOffset: 0 };
  if (isPairedVoipOpusAt(payload, 1)) {
    return { packet: payload.subarray(1), prefixOffset: 1 };
  }
  const maxOffset = Math.min(8, payload.length - 3);
  for (let offset = 2; offset <= maxOffset; offset++) {
    if (isPairedVoipOpusAt(payload, offset)) {
      return { packet: payload.subarray(offset), prefixOffset: offset };
    }
  }
  if (isSingleVoipOpusAt(payload, 1)) {
    return { packet: payload.subarray(1), prefixOffset: 1 };
  }
  for (
    let offset = 2;
    offset <= Math.min(8, payload.length - 1);
    offset++
  ) {
    if (isSingleVoipOpusAt(payload, offset)) {
      return { packet: payload.subarray(offset), prefixOffset: offset };
    }
  }
  return { packet: payload.subarray(1), prefixOffset: 1 };
}

export async function startCallAudioIO(options: AudioOptions): Promise<{
  stop(): Promise<void>;
  done: Promise<void>;
}> {
  options.signal.throwIfAborted();
  let cancelLoading: (() => void) | undefined;
  let codec: CodecFactory;
  try {
    codec = await new Promise<CodecFactory>((resolve, reject) => {
      cancelLoading = () => reject(options.signal.reason);
      options.signal.addEventListener("abort", cancelLoading, { once: true });
      options.codecFactory().then(resolve, reject);
    });
  } finally {
    if (cancelLoading) {
      options.signal.removeEventListener("abort", cancelLoading);
    }
  }
  options.signal.throwIfAborted();
  const encoder = codec.newEncoder({
    sampleRate,
    channels: 1,
    bitrate: 32_000,
    frameDurationMs: 20,
    signal: options.opusSignal ?? "voice",
    vbr: true,
  });
  let decoder: ReturnType<CodecFactory["newDecoder"]>;
  let devices: AudioDevices;
  try {
    decoder = codec.newDecoder({ sampleRate, channels: 1 });
    try {
      options.signal.throwIfAborted();
      devices = options.spawnDevices?.() ?? spawnAudioDevices(
        options.inputDevice ?? "default",
        options.outputDevice ?? "default",
      );
    } catch (error) {
      decoder.close?.();
      throw error;
    }
  } catch (error) {
    encoder.close?.();
    throw error;
  }

  const cancellation = new AbortController();
  const reader = devices.input.getReader();
  const writer = devices.output.getWriter();
  let pending = new Uint8Array(0);
  const receiveQueue: QueuedAudioSegment[] = [];
  let queuedSamples = 0;
  let firstDecodedWritten = false;
  let closing: Promise<void> | undefined;
  const startedAt = performance.now();
  let nextReportAt = startedAt + 5000;
  const stats = {
    capturedBytes: 0,
    capturedSamples: 0,
    discardedCaptureSamples: 0,
    sentPackets: 0,
    underflowFrames: 0,
    lastCapturedPeak: 0,
    lastCapturedRmsDbfs: -120,
    lastEncodedInputPeak: 0,
    lastEncodedInputRmsDbfs: -120,
    rxPackets: 0,
    rxBytes: 0,
    rxNonStandardPrefixPackets: 0,
    rxMultiBytePrefixPackets: 0,
    decodedFrames: 0,
    decodedSamples: 0,
    decodeErrors: 0,
    discardedReceiveSamples: 0,
    speakerUnderflowFrames: 0,
    writtenFrames: 0,
    writtenSamples: 0,
    lastDecodedPeak: 0,
    lastDecodedRmsDbfs: -120,
  };
  function clearReceiveQueue(countDiscarded = false) {
    if (countDiscarded) stats.discardedReceiveSamples += queuedSamples;
    receiveQueue.length = 0;
    queuedSamples = 0;
  }
  function enqueueDecoded(samples: Int16Array, now: number) {
    if (!samples.length) return;
    if (options.controls.deafened) {
      clearReceiveQueue();
      return;
    }
    const owned = samples.slice();
    receiveQueue.push({ samples: owned, offset: 0, receivedAt: now });
    queuedSamples += owned.length;
    while (queuedSamples > maxQueueSamples && receiveQueue.length > 0) {
      const head = receiveQueue[0];
      const available = head.samples.length - head.offset;
      const excess = queuedSamples - maxQueueSamples;
      if (available <= excess) {
        receiveQueue.shift();
        queuedSamples -= available;
        stats.discardedReceiveSamples += available;
      } else {
        head.offset += excess;
        queuedSamples -= excess;
        stats.discardedReceiveSamples += excess;
      }
    }
  }
  function takeSpeakerQuantum(now: number): {
    samples: Int16Array;
    hasDecodedAudio: boolean;
  } {
    const out = new Int16Array(frameSamples);
    if (options.controls.deafened) {
      clearReceiveQueue();
      stats.speakerUnderflowFrames++;
      return { samples: out, hasDecodedAudio: false };
    }
    while (
      receiveQueue.length > 0 &&
      now - receiveQueue[0].receivedAt > maxSegmentAgeMs
    ) {
      const expired = receiveQueue.shift()!;
      const dropped = expired.samples.length - expired.offset;
      queuedSamples -= dropped;
      stats.discardedReceiveSamples += dropped;
    }
    let filled = 0;
    while (filled < frameSamples && receiveQueue.length > 0) {
      const head = receiveQueue[0];
      const available = head.samples.length - head.offset;
      const take = Math.min(frameSamples - filled, available);
      out.set(head.samples.subarray(head.offset, head.offset + take), filled);
      filled += take;
      head.offset += take;
      queuedSamples -= take;
      if (head.offset >= head.samples.length) {
        receiveQueue.shift();
      }
    }
    if (filled < frameSamples) {
      stats.speakerUnderflowFrames++;
    }
    return { samples: out, hasDecodedAudio: filled > 0 };
  }
  function reportAudio(
    event:
      | "interval"
      | "stopped"
      | "first_decode_error"
      | "first_decoded_audio"
      | "first_audio_written",
  ) {
    console.error(
      "[call audio stats]",
      JSON.stringify({
        event,
        elapsedMs: Math.round(performance.now() - startedAt),
        ...stats,
        muted: options.controls.muted,
        micGain: options.controls.micGain,
        deafened: options.controls.deafened,
        spkGain: options.controls.spkGain,
        ...(event === "first_decode_error"
          ? { reason: "opus_decode_rejected" }
          : {}),
      }),
    );
  }
  const aborted = new Promise<never>((_, reject) => {
    cancellation.signal.addEventListener("abort", () => {
      reject(new DOMException("Audio stopped", "AbortError"));
    }, { once: true });
  });
  void aborted.catch(() => {});
  const untilStopped = <T>(promise: Promise<T>) =>
    Promise.race([promise, aborted]);
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
    clearReceiveQueue();
    options.signal.removeEventListener("abort", parentAbort);
    void writer.abort().catch(() => {});
    closing = Promise.allSettled([
      reader.cancel(),
      devices.close(),
    ]).then(() => {
      encoder.close?.();
      decoder.close?.();
      try {
        reader.releaseLock();
      } catch { /* stream may still hold lock if canceled late */ }
      try {
        writer.releaseLock();
      } catch { /* stream may still hold lock if write hung */ }
      reportAudio("stopped");
    });
    await closing;
  }
  function parentAbort() {
    void stop();
  }
  options.signal.addEventListener("abort", parentAbort, { once: true });
  if (options.signal.aborted) parentAbort();

  async function capture() {
    while (!cancellation.signal.aborted) {
      const { value, done } = await untilStopped(reader.read());
      if (done) throw new Error("Microphone capture ended");
      const bytes = new Uint8Array(pending.length + value.length);
      bytes.set(pending);
      bytes.set(value, pending.length);
      stats.capturedBytes += value.length;
      stats.capturedSamples = Math.floor(stats.capturedBytes / 2);
      const level = signalLevel(
        new Int16Array(
          value.buffer,
          value.byteOffset,
          Math.floor(value.byteLength / 2),
        ),
      );
      stats.lastCapturedPeak = level.peak;
      stats.lastCapturedRmsDbfs = level.rmsDbfs;
      const discard = Math.max(
        0,
        Math.floor((bytes.length - frameBytes * 10) / 2) * 2,
      );
      pending = bytes.subarray(discard);
      stats.discardedCaptureSamples += discard / 2;
    }
  }
  async function send() {
    let next = performance.now();
    let pendingFirstPacket: Uint8Array | undefined;
    while (!cancellation.signal.aborted) {
      if (performance.now() >= nextReportAt) {
        reportAudio("interval");
        nextReportAt = performance.now() + 5000;
      }
      let samples = new Int16Array(frameSamples);
      if (pending.length >= frameBytes) {
        const frame = pending.slice(0, frameBytes);
        pending = pending.subarray(frameBytes);
        samples = new Int16Array(frame.buffer);
      } else {
        stats.underflowFrames++;
      }
      if (options.controls.muted) samples.fill(0);
      const inputSamples = gain(samples, options.controls.micGain);
      const inputLevel = signalLevel(inputSamples);
      stats.lastEncodedInputPeak = inputLevel.peak;
      stats.lastEncodedInputRmsDbfs = inputLevel.rmsDbfs;
      const packet = encoder.encode({
        samples: inputSamples,
        sampleRate,
        channels: 1,
      });
      if (packet) {
        if (options.pairedOpus) {
          const single = new Uint8Array(packet.length + 1);
          single.set(packet, 1);
          if (!pendingFirstPacket) {
            pendingFirstPacket = single;
          } else {
            const [paired] = packetizeNativeGroupOpusPairs([
              pendingFirstPacket,
              single,
            ]);
            pendingFirstPacket = undefined;
            if (paired) {
              paired[0] = stats.sentPackets < 2 ? 0x00 : 0x10;
              await untilStopped(
                options.transport.send(paired, {
                  timestampStep: frameSamples * 2,
                }),
              );
              stats.sentPackets++;
            }
          }
        } else {
          const payload = new Uint8Array(packet.length + 1);
          payload[0] = stats.sentPackets < 2 ? 0x00 : 0x10;
          payload.set(packet, 1);
          await untilStopped(
            options.transport.send(payload, { timestampStep: frameSamples }),
          );
          stats.sentPackets++;
        }
      }
      next = Math.max(next + 20, performance.now());
      await pause(Math.max(0, next - performance.now()));
    }
  }
  async function receive() {
    const iterator = options.transport.receive()[Symbol.asyncIterator]();
    while (!cancellation.signal.aborted) {
      const item = await untilStopped(iterator.next());
      if (item.done) return;
      const payload = item.value;
      stats.rxPackets++;
      stats.rxBytes += payload.length;
      if (!payload.length) continue;
      if (payload[0] !== 0 && payload[0] !== 0x10) {
        stats.rxNonStandardPrefixPackets++;
      }
      const { packet, prefixOffset } = extractPlanetOpusPacket(payload);
      if (prefixOffset > 1) {
        stats.rxMultiBytePrefixPackets++;
      }
      let frame;
      try {
        frame = decoder.decode(packet);
      } catch {
        stats.decodeErrors++;
        if (stats.decodeErrors === 1) reportAudio("first_decode_error");
        continue;
      }
      if (!frame) continue;
      stats.decodedFrames++;
      stats.decodedSamples += frame.samples.length;
      const level = signalLevel(frame.samples);
      stats.lastDecodedPeak = level.peak;
      stats.lastDecodedRmsDbfs = level.rmsDbfs;
      if (stats.decodedFrames === 1) reportAudio("first_decoded_audio");
      enqueueDecoded(frame.samples, performance.now());
    }
  }
  async function playout() {
    let next = performance.now();
    while (!cancellation.signal.aborted) {
      const { samples, hasDecodedAudio } = takeSpeakerQuantum(
        performance.now(),
      );
      const outputSamples = gain(samples, options.controls.spkGain);
      await untilStopped(
        writer.write(
          new Uint8Array(
            outputSamples.buffer,
            outputSamples.byteOffset,
            outputSamples.byteLength,
          ),
        ),
      );
      stats.writtenFrames++;
      stats.writtenSamples += outputSamples.length;
      if (hasDecodedAudio && !firstDecodedWritten) {
        firstDecodedWritten = true;
        reportAudio("first_audio_written");
      }
      next = Math.max(next + 20, performance.now());
      await pause(Math.max(0, next - performance.now()));
    }
  }
  async function run(task: () => Promise<void>, eof = false) {
    try {
      await task();
      if (eof && !cancellation.signal.aborted) {
        options.onEnded?.();
        await stop();
      }
    } catch (error) {
      if (!cancellation.signal.aborted) {
        options.onEnded?.(
          error instanceof Error ? error : new Error(String(error)),
        );
        await stop();
      }
    }
  }
  const done = Promise.all([
    run(capture),
    run(send),
    run(receive, true),
    run(playout),
  ]).then(() => closing).then(() => {});
  return { stop, done };
}
