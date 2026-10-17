import type { CodecFactory } from "../vendor/linejs-call/audio.ts";

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
  spawnDevices?: () => AudioDevices;
  onEnded?: (error?: Error) => void;
};

const sampleRate = 48_000;
const frameSamples = 960;
const frameBytes = frameSamples * 2;

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
    frameDurationMs: 20,
    signal: options.opusSignal ?? "music",
    vbr: false,
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
  let closing: Promise<void> | undefined;
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
    options.signal.removeEventListener("abort", parentAbort);
    closing = Promise.allSettled([
      reader.cancel(),
      writer.abort(),
      devices.close(),
    ]).then(() => {
      encoder.close?.();
      decoder.close?.();
      reader.releaseLock();
      writer.releaseLock();
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
      const discard = Math.max(
        0,
        Math.floor((bytes.length - frameBytes * 10) / 2) * 2,
      );
      pending = bytes.subarray(discard);
    }
  }
  async function send() {
    let next = performance.now();
    while (!cancellation.signal.aborted) {
      let samples = new Int16Array(frameSamples);
      if (pending.length >= frameBytes) {
        const frame = pending.slice(0, frameBytes);
        pending = pending.subarray(frameBytes);
        samples = new Int16Array(frame.buffer);
      }
      if (options.controls.muted) samples.fill(0);
      const packet = encoder.encode({
        samples: gain(samples, options.controls.micGain),
        sampleRate,
        channels: 1,
      });
      if (packet) {
        const payload = new Uint8Array(packet.length + 1);
        payload.set(packet, 1);
        await untilStopped(
          options.transport.send(payload, { timestampStep: frameSamples }),
        );
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
      if (!payload.length) continue;
      const packet = payload[0] === 0 || payload[0] === 0x10
        ? payload.subarray(1)
        : payload;
      let frame;
      try {
        frame = decoder.decode(packet);
      } catch {
        continue;
      }
      if (!frame || options.controls.deafened) continue;
      const samples = gain(frame.samples, options.controls.spkGain);
      await untilStopped(writer.write(
        new Uint8Array(
          samples.buffer,
          samples.byteOffset,
          samples.byteLength,
        ),
      ));
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
  ]).then(() => closing).then(() => {});
  return { stop, done };
}
