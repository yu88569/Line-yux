import { assert, assertEquals } from "@std/assert";
import { startCallAudioIO } from "../src/call_audio.ts";
import { opusCodecFactory } from "../vendor/linejs-call/opus.ts";

async function pactl(...args: string[]): Promise<string> {
  const result = await new Deno.Command("pactl", {
    args,
    stdout: "piped",
    stderr: "piped",
  }).output();
  if (!result.success) throw new Error(new TextDecoder().decode(result.stderr));
  return new TextDecoder().decode(result.stdout).trim();
}

function parseList(text: string): unknown[] {
  const value: unknown = JSON.parse(text);
  assert(Array.isArray(value), "pactl must return a JSON list");
  return value;
}

async function sinkIndex(name: string): Promise<number> {
  for (
    const value of parseList(await pactl("--format=json", "list", "sinks"))
  ) {
    if (
      value && typeof value === "object" && "name" in value &&
      value.name === name &&
      "index" in value && typeof value.index === "number"
    ) return value.index;
  }
  throw new Error(`Temporary sink not found: ${name}`);
}

const prefix = `line_gtk_check_${Deno.pid}_${Date.now()}`;
const inputName = `${prefix}_input`;
const outputName = `${prefix}_output`;
const modules: string[] = [];
const children: Deno.ChildProcess[] = [];
let session: Awaited<ReturnType<typeof startCallAudioIO>> | undefined;
let packets = 0;
let cleanupErrors: unknown[] = [];
let ended: Error | undefined;
let incoming!: ReadableStreamDefaultController<Uint8Array>;
const received = new ReadableStream<Uint8Array>({
  start(controller) {
    incoming = controller;
  },
}).getReader();

try {
  for (const name of [inputName, outputName]) {
    modules.push(
      await pactl(
        "load-module",
        "module-null-sink",
        `sink_name=${name}`,
        "rate=48000",
        "channels=1",
      ),
    );
  }
  const target = await sinkIndex(outputName);
  const existingStreams = new Set(
    parseList(await pactl("--format=json", "list", "sink-inputs"))
      .map((value) =>
        value && typeof value === "object" && "index" in value
          ? value.index
          : undefined
      ),
  );
  session = await startCallAudioIO({
    inputDevice: `${inputName}.monitor`,
    outputDevice: outputName,
    signal: new AbortController().signal,
    controls: { muted: false, deafened: false, micGain: 1, spkGain: 1 },
    codecFactory: opusCodecFactory,
    onEnded(error) {
      ended = error ?? new Error("Audio receive ended");
    },
    transport: {
      send(payload) {
        packets++;
        incoming.enqueue(payload);
        return Promise.resolve();
      },
      receive() {
        return {
          [Symbol.asyncIterator]() {
            return {
              async next(): Promise<IteratorResult<Uint8Array>> {
                const result = await received.read();
                return result.done
                  ? { done: true, value: undefined }
                  : { done: false, value: result.value };
              },
            };
          },
        };
      },
    },
  });
  let actualSink: unknown;
  const deadline = Date.now() + 8000;
  while (actualSink === undefined && Date.now() < deadline) {
    if (ended) throw ended;
    for (
      const value of parseList(
        await pactl("--format=json", "list", "sink-inputs"),
      )
    ) {
      if (
        !value || typeof value !== "object" || !("properties" in value) ||
        !value.properties || typeof value.properties !== "object"
      ) continue;
      if ("index" in value && existingStreams.has(value.index)) continue;
      const properties = value.properties;
      if (
        !("media.name" in properties) ||
        (properties["media.name"] !== outputName &&
          properties["media.name"] !== "LINE GTK call")
      ) continue;
      if ("sink" in value) actualSink = value.sink;
    }
    if (actualSink === undefined) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
  assertEquals(
    actualSink,
    target,
    "Call playback must use the selected null sink",
  );

  const tone = new Deno.Command("ffmpeg", {
    args: [
      "-hide_banner",
      "-loglevel",
      "error",
      "-re",
      "-f",
      "lavfi",
      "-i",
      "sine=frequency=440:sample_rate=48000",
      "-t",
      "5",
      "-f",
      "pulse",
      "-device",
      inputName,
      `${prefix}_tone`,
    ],
    stdin: "null",
    stdout: "null",
    stderr: "piped",
  }).spawn();
  children.push(tone);
  const toneError = tone.stderr.pipeTo(new WritableStream());
  const monitor = new Deno.Command("ffmpeg", {
    args: [
      "-hide_banner",
      "-loglevel",
      "error",
      "-f",
      "pulse",
      "-i",
      `${outputName}.monitor`,
      "-t",
      "4",
      "-ac",
      "1",
      "-ar",
      "48000",
      "-f",
      "s16le",
      "pipe:1",
    ],
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  children.push(monitor);
  const timeout = setTimeout(() => {
    try {
      monitor.kill("SIGTERM");
    } catch { /* monitor already exited */ }
  }, 10000);
  const captured = await monitor.output().finally(() => clearTimeout(timeout));
  assert(captured.success, new TextDecoder().decode(captured.stderr));
  const samples = new Int16Array(
    captured.stdout.buffer,
    captured.stdout.byteOffset,
    Math.floor(captured.stdout.length / 2),
  );
  let peak = 0;
  for (const sample of samples) peak = Math.max(peak, Math.abs(sample));
  assert(
    peak > 500,
    `Synthetic audio must reach selected sink monitor; peak=${peak}`,
  );
  if (ended) throw ended;
  console.log(
    JSON.stringify({
      selectedSinkConfirmed: true,
      packets,
      decodedPcmBytes: captured.stdout.length,
      decodedPeak: peak,
    }),
  );
  try {
    tone.kill("SIGTERM");
  } catch { /* tone already ended */ }
  await toneError;
} finally {
  await session?.stop();
  await session?.done;
  for (const child of children) {
    try {
      child.kill("SIGTERM");
    } catch { /* child already exited */ }
  }
  await Promise.allSettled(children.map((child) => child.status));
  const unloaded = await Promise.allSettled(
    modules.reverse().map((module) => pactl("unload-module", module)),
  );
  for (const result of unloaded) {
    if (result.status === "rejected") {
      console.error("Temporary sink cleanup failed", result.reason);
    }
  }
  await received.cancel();
  received.releaseLock();
  cleanupErrors = unloaded.flatMap((result) =>
    result.status === "rejected" ? [result.reason] : []
  );
}
if (cleanupErrors.length) {
  throw new AggregateError(cleanupErrors, "Temporary sink cleanup failed");
}
