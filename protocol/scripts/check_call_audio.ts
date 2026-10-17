import { startCallAudioIO } from "../src/call_audio.ts";
import { opusCodecFactory } from "../vendor/linejs-call/opus.ts";
const microphone = new Deno.Command("ffmpeg", {
  args: [
    "-hide_banner",
    "-loglevel",
    "error",
    "-re",
    "-f",
    "lavfi",
    "-i",
    "sine=frequency=440:sample_rate=48000",
    "-ac",
    "1",
    "-f",
    "s16le",
    "pipe:1",
  ],
  stdin: "null",
  stdout: "piped",
  stderr: "null",
}).spawn();
const speaker = new Deno.Command("ffmpeg", {
  args: [
    "-hide_banner",
    "-loglevel",
    "error",
    "-f",
    "s16le",
    "-ar",
    "48000",
    "-ac",
    "1",
    "-i",
    "pipe:0",
    "-f",
    "null",
    "-",
  ],
  stdin: "piped",
  stdout: "null",
  stderr: "null",
}).spawn();
let queued!: ReadableStreamDefaultController<Uint8Array>;
const incoming = new ReadableStream<Uint8Array>({
  start(c) {
    queued = c;
  },
}).getReader();
let packets = 0, bytes = 0, peak = 0;
const output = speaker.stdin.getWriter();
const session = await startCallAudioIO({
  transport: {
    send(packet) {
      packets++;
      queued.enqueue(packet);
      return Promise.resolve();
    },
    receive() {
      return {
        [Symbol.asyncIterator]() {
          return {
            async next(): Promise<IteratorResult<Uint8Array>> {
              const x = await incoming.read();
              return x.done
                ? { done: true, value: undefined }
                : { done: false, value: x.value };
            },
          };
        },
      };
    },
  },
  codecFactory: opusCodecFactory,
  signal: new AbortController().signal,
  controls: { muted: false, deafened: false, micGain: 1, spkGain: 1 },
  spawnDevices() {
    return {
      input: microphone.stdout,
      output: new WritableStream({
        async write(chunk) {
          bytes += chunk.length;
          for (const x of new Int16Array(chunk.slice().buffer)) {
            peak = Math.max(peak, Math.abs(x));
          }
          await output.write(chunk);
        },
        async abort() {
          await output.abort();
        },
      }),
      async close() {
        for (const p of [microphone, speaker]) {
          try {
            p.kill("SIGTERM");
          } catch { /* already exited */ }
        }
        await Promise.all([microphone.status, speaker.status]);
      },
    };
  },
});
await new Promise((r) => setTimeout(r, 500));
await session.stop();
await session.done;
if (packets < 10 || bytes < 19200 || peak < 500) {
  throw new Error(JSON.stringify({ packets, bytes, peak }));
}
console.log(
  JSON.stringify({
    packets,
    decodedPcmBytes: bytes,
    decodedPeak: peak,
    ffmpegProcessesStopped: true,
  }),
);
