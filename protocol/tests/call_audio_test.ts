import { assert, assertEquals, assertRejects } from "@std/assert";
import { type AudioDevices, startCallAudioIO } from "../src/call_audio.ts";
import { opusCodecFactory } from "../vendor/linejs-call/opus.ts";

const controls = { muted: false, deafened: false, micGain: 1, spkGain: 1 };

function fixture() {
  let input!: ReadableStreamDefaultController<Uint8Array>;
  let incoming!: ReadableStreamDefaultController<Uint8Array>;
  const output: Uint8Array[] = [];
  let closed = 0;
  const devices: AudioDevices = {
    input: new ReadableStream({
      start(controller) {
        input = controller;
      },
    }),
    output: new WritableStream({
      write(bytes) {
        output.push(bytes.slice());
      },
    }),
    close() {
      closed++;
      return Promise.resolve();
    },
  };
  const received = new ReadableStream<Uint8Array>({
    start(controller) {
      incoming = controller;
    },
  });
  const reader = received.getReader();
  const sent: Uint8Array[] = [];
  const transport = {
    send(payload: Uint8Array, options: { timestampStep: number }) {
      assertEquals(options.timestampStep, 960);
      sent.push(payload.slice());
      return Promise.resolve();
    },
    receive() {
      return {
        [Symbol.asyncIterator]() {
          return {
            async next(): Promise<IteratorResult<Uint8Array>> {
              const result = await reader.read();
              return result.done
                ? { done: true, value: undefined }
                : { done: false, value: result.value };
            },
          };
        },
      };
    },
  };
  return {
    input: () => input,
    incoming: () => incoming,
    devices,
    output,
    sent,
    transport,
    closed: () => closed,
  };
}

async function waitFor(predicate: () => boolean) {
  const deadline = Date.now() + 2000;
  while (!predicate()) {
    if (Date.now() > deadline) {
      throw new Error("Audio pipeline did not produce expected frames");
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

Deno.test("audio abort during codec loading never opens microphone or sends", async () => {
  const f = fixture();
  const abort = new AbortController();
  let resolve!: (codec: Awaited<ReturnType<typeof opusCodecFactory>>) => void;
  const codec = await opusCodecFactory();
  let spawned = 0;
  const starting = startCallAudioIO({
    transport: f.transport,
    controls,
    signal: abort.signal,
    codecFactory: () =>
      new Promise((done) => {
        resolve = done;
      }),
    spawnDevices() {
      spawned++;
      return f.devices;
    },
  });
  abort.abort();
  resolve(codec);
  await assertRejects(() => starting, DOMException);
  assertEquals(spawned, 0);
  assertEquals(f.sent.length, 0);
});

Deno.test("audio startup failure releases both real codec instances", async () => {
  const codec = await opusCodecFactory();
  let closed = 0;
  const wrapClose = <T extends { close?: () => void }>(instance: T): T => {
    const close = instance.close;
    instance.close = () => {
      closed++;
      close?.();
    };
    return instance;
  };
  await assertRejects(
    () =>
      startCallAudioIO({
        transport: fixture().transport,
        controls,
        signal: new AbortController().signal,
        codecFactory: () =>
          Promise.resolve({
            newEncoder: (options) => wrapClose(codec.newEncoder(options)),
            newDecoder: (options) => wrapClose(codec.newDecoder(options)),
          }),
        spawnDevices() {
          throw new Error("speaker unavailable");
        },
      }),
    Error,
    "speaker unavailable",
  );
  assertEquals(closed, 2);
});

Deno.test("real Opus carries generated speech both ways and short silence packets", async () => {
  const f = fixture();
  const codec = await opusCodecFactory();
  const peerEncoder = codec.newEncoder({
    sampleRate: 48000,
    channels: 1,
    frameDurationMs: 20,
  });
  const peerDecoder = codec.newDecoder({ sampleRate: 48000, channels: 1 });
  const samples = Int16Array.from(
    { length: 9600 },
    (_, i) => Math.round(8000 * Math.sin(2 * Math.PI * 440 * i / 48000)),
  );
  const session = await startCallAudioIO({
    transport: f.transport,
    controls,
    signal: new AbortController().signal,
    codecFactory: opusCodecFactory,
    spawnDevices: () => f.devices,
  });
  try {
    const bytes = new Uint8Array(samples.buffer);
    // Split mid-sample, as allowed by a byte stream.
    f.input().enqueue(bytes.slice(0, 123));
    f.input().enqueue(bytes.slice(123));
    const encoded = peerEncoder.encode({
      samples: samples.slice(0, 960),
      sampleRate: 48000,
      channels: 1,
    });
    assert(encoded);
    const payload = new Uint8Array(encoded.length + 1);
    payload.set(encoded, 1);
    f.incoming().enqueue(payload);
    f.incoming().enqueue(new Uint8Array([0, 0xf8, 0xff, 0xfe]));
    await waitFor(() => f.output.length === 2 && f.sent.length >= 4);
    assertEquals(f.output.map((bytes) => bytes.length), [1920, 1920]);
    assert(
      new Int16Array(f.output[0].slice().buffer).some((value) =>
        Math.abs(value) > 500
      ),
    );
    assert(
      f.sent.some((payload) =>
        peerDecoder.decode(payload.subarray(1))?.samples.some((value) =>
          Math.abs(value) > 500
        )
      ),
    );
  } finally {
    await session.stop();
    await session.done;
    peerEncoder.close?.();
    peerDecoder.close?.();
  }
  const count = f.sent.length;
  await new Promise((resolve) => setTimeout(resolve, 30));
  assertEquals(f.sent.length, count);
  assertEquals(f.closed(), 1);
});

Deno.test("microphone failure reports an error and stops audio", async () => {
  const f = fixture();
  let ended: Error | undefined;
  const session = await startCallAudioIO({
    transport: f.transport,
    controls,
    signal: new AbortController().signal,
    codecFactory: opusCodecFactory,
    spawnDevices: () => f.devices,
    onEnded(error) {
      ended = error;
    },
  });
  f.input().close();
  await session.done;
  assertEquals(ended?.message, "Microphone capture ended");
  assertEquals(f.closed(), 1);
});

Deno.test("audio abort while devices open closes session without sending", async () => {
  const f = fixture();
  const abort = new AbortController();
  const session = await startCallAudioIO({
    transport: f.transport,
    controls,
    signal: abort.signal,
    codecFactory: opusCodecFactory,
    spawnDevices() {
      abort.abort();
      return f.devices;
    },
  });
  await session.done;
  assertEquals(f.sent.length, 0);
  assertEquals(f.closed(), 1);
});

Deno.test("stopping audio exits blocked transport send and receive", async () => {
  const f = fixture();
  let attempted = false;
  const session = await startCallAudioIO({
    transport: {
      send() {
        attempted = true;
        return new Promise<void>(() => {});
      },
      receive: () => f.transport.receive(),
    },
    controls,
    signal: new AbortController().signal,
    codecFactory: opusCodecFactory,
    spawnDevices: () => f.devices,
  });
  assert(attempted);
  await session.stop();
  await session.done;
  assertEquals(f.closed(), 1);
});

Deno.test("audio abort promptly interrupts codec loading that never settles", async () => {
  const f = fixture();
  const abort = new AbortController();
  let spawned = false;
  const starting = startCallAudioIO({
    transport: f.transport,
    controls,
    signal: abort.signal,
    codecFactory: () => new Promise(() => {}),
    spawnDevices() {
      spawned = true;
      return f.devices;
    },
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error("Codec startup did not abort within 100ms")),
      100,
    );
  });
  abort.abort();
  try {
    await assertRejects(() => Promise.race([starting, deadline]), DOMException);
  } finally {
    clearTimeout(timer);
  }
  assertEquals(spawned, false);
  assertEquals(f.sent.length, 0);
});
