import { assert, assertEquals, assertRejects } from "@std/assert";
import {
  type AudioDevices,
  extractPlanetOpusPacket,
  startCallAudioIO,
} from "../src/call_audio.ts";
import { packetizeNativeGroupOpusPairs } from "../vendor/linejs-call/audio.ts";
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
    await waitFor(() =>
      f.output.some((bytes) =>
        new Int16Array(bytes.slice().buffer).some((value) =>
          Math.abs(value) > 500
        )
      ) && f.sent.length >= 4
    );
    assert(f.output.every((bytes) => bytes.length === 1920));
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
  const outCount = f.output.length;
  await new Promise((resolve) => setTimeout(resolve, 30));
  assertEquals(f.sent.length, count);
  assertEquals(f.output.length, outCount);
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

Deno.test("audio diagnostics distinguish rejected Opus, decoded audio and speaker writes without packet contents", async () => {
  const f = fixture();
  const codec = await opusCodecFactory();
  const encoder = codec.newEncoder({ sampleRate: 48000, channels: 1 });
  const logs: string[] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => {
    if (args[0] === "[call audio stats]") logs.push(String(args[1]));
  };
  let session: Awaited<ReturnType<typeof startCallAudioIO>> | undefined;
  try {
    session = await startCallAudioIO({
      transport: f.transport,
      controls,
      signal: new AbortController().signal,
      codecFactory: opusCodecFactory,
      spawnDevices: () => f.devices,
    });
    const malformed = new Uint8Array(4097).fill(0xff);
    malformed.set(new TextEncoder().encode("PRIVATE_AUDIO_PACKET"), 100);
    for (let i = 0; i < 10; i++) f.incoming().enqueue(malformed);
    const samples = Int16Array.from(
      { length: 960 },
      (_, i) => Math.round(8000 * Math.sin(2 * Math.PI * 440 * i / 48000)),
    );
    const encoded = encoder.encode({ samples, sampleRate: 48000, channels: 1 });
    assert(encoded);
    const payload = new Uint8Array(encoded.length + 1);
    payload.set(encoded, 1);
    const captured = new Int16Array(9600);
    for (let offset = 0; offset < captured.length; offset += samples.length) {
      captured.set(samples, offset);
    }
    f.input().enqueue(new Uint8Array(captured.buffer));
    f.incoming().enqueue(payload);
    await waitFor(() =>
      logs.some((log) => log.includes('"event":"first_audio_written"')) &&
      f.sent.length >= 2
    );
    assertEquals(logs.length, 3);
    assert(logs.some((log) => log.includes('"event":"first_decode_error"')));
    assert(logs.some((log) => log.includes('"event":"first_decoded_audio"')));
    assert(logs.some((log) => log.includes('"event":"first_audio_written"')));
    await session.stop();
    await session.done;
    await session.stop();
    assertEquals(logs.length, 4);
    const summary = logs[3];
    assert(summary.includes('"event":"stopped"'));
    assert(summary.includes('"rxPackets":11'));
    assert(summary.includes('"decodeErrors":10'));
    assert(summary.includes('"decodedFrames":1'));
    assert(summary.includes('"capturedSamples":9600'));
    assert(summary.includes('"lastCapturedPeak":8000'));
    assert(summary.includes('"lastEncodedInputPeak":8000'));
    assert(summary.includes('"underflowFrames":1'));
    const summaryData: unknown = JSON.parse(summary);
    assert(summaryData && typeof summaryData === "object");
    assert(
      "writtenFrames" in summaryData &&
        typeof summaryData.writtenFrames === "number" &&
        summaryData.writtenFrames >= 1 &&
        "writtenSamples" in summaryData &&
        summaryData.writtenSamples === summaryData.writtenFrames * 960,
    );
    assert(
      "sentPackets" in summaryData &&
        typeof summaryData.sentPackets === "number" &&
        summaryData.sentPackets >= 2,
    );
    assert(!logs.join(" ").includes("PRIVATE_AUDIO_PACKET"));
    assert(!logs.join(" ").includes("samples.buffer"));
  } finally {
    await session?.stop();
    await session?.done;
    encoder.close?.();
    console.error = originalError;
  }
});

Deno.test("continuous speaker playout slices 20/40/60ms decoded packets across fixed 20ms quanta and zero-fills gaps", async () => {
  const f = fixture();
  const decodedLengths = [960, 1920, 2880];
  let decodeIndex = 0;
  const session = await startCallAudioIO({
    transport: f.transport,
    controls: { ...controls },
    signal: new AbortController().signal,
    codecFactory: () =>
      Promise.resolve({
        newEncoder: () => ({ encode: () => new Uint8Array([1, 2, 3]) }),
        newDecoder: () => ({
          decode: () => {
            const len = decodedLengths[decodeIndex] ?? 960;
            const tag = (decodeIndex + 1) * 1000;
            decodeIndex++;
            const samples = new Int16Array(len);
            for (let i = 0; i < len; i++) {
              samples[i] = tag + Math.floor(i / 960) + 1;
            }
            return { samples, sampleRate: 48000, channels: 1 };
          },
        }),
      }),
    spawnDevices: () => f.devices,
  });
  try {
    await waitFor(() => f.output.length >= 1);
    assert(new Int16Array(f.output[0].slice().buffer).every((v) => v === 0));
    f.incoming().enqueue(new Uint8Array([0, 1]));
    f.incoming().enqueue(new Uint8Array([0, 2]));
    f.incoming().enqueue(new Uint8Array([0, 3]));
    const observedMarkers: number[] = [];
    await waitFor(() => {
      observedMarkers.length = 0;
      for (const bytes of f.output) {
        assertEquals(bytes.length, 1920);
        const first = new Int16Array(bytes.slice().buffer)[0];
        if (first !== 0) observedMarkers.push(first);
      }
      return observedMarkers.length >= 6;
    });
    assertEquals(observedMarkers.slice(0, 6), [
      1001,
      2001,
      2002,
      3001,
      3002,
      3003,
    ]);
  } finally {
    await session.stop();
    await session.done;
  }
});

Deno.test("speaker playout enforces 200ms queue cap, drops stale segments after blocked writer, and flushes on deafen", async () => {
  let incoming!: ReadableStreamDefaultController<Uint8Array>;
  const output: Int16Array[] = [];
  let holdWrite = false;
  let releaseWrite: (() => void) | undefined;
  let inFlightWrites = 0;
  let maxConcurrentWrites = 0;
  const devices: AudioDevices = {
    input: new ReadableStream(),
    output: new WritableStream({
      async write(bytes) {
        inFlightWrites++;
        maxConcurrentWrites = Math.max(maxConcurrentWrites, inFlightWrites);
        try {
          if (holdWrite) {
            await new Promise<void>((resolve) => {
              releaseWrite = resolve;
            });
          }
          const copy = bytes.slice();
          output.push(new Int16Array(copy.buffer));
        } finally {
          inFlightWrites--;
        }
      },
    }),
    close: () => Promise.resolve(),
  };
  const received = new ReadableStream<Uint8Array>({
    start(controller) {
      incoming = controller;
    },
  });
  const reader = received.getReader();
  const ctl = { muted: false, deafened: false, micGain: 1, spkGain: 2 };
  const session = await startCallAudioIO({
    transport: {
      send: () => Promise.resolve(),
      receive: () => ({
        [Symbol.asyncIterator]: () => ({
          async next() {
            const res = await reader.read();
            return res.done
              ? { done: true, value: undefined }
              : { done: false, value: res.value };
          },
        }),
      }),
    },
    controls: ctl,
    signal: new AbortController().signal,
    codecFactory: () =>
      Promise.resolve({
        newEncoder: () => ({ encode: () => new Uint8Array([1]) }),
        newDecoder: () => ({
          decode: (pkt) => {
            const val = pkt[0];
            const samples = new Int16Array(960).fill(val);
            return { samples, sampleRate: 48000, channels: 1 };
          },
        }),
      }),
    spawnDevices: () => devices,
  });
  try {
    // Overflow cap: enqueue 15 frames (300ms), only newest 10 (values 6..15, scaled x2 -> 12..30) can survive.
    for (let i = 1; i <= 15; i++) {
      incoming.enqueue(new Uint8Array([0, i]));
    }
    await waitFor(() => output.some((q) => q[0] === 30));
    const nonZeroBeforeStall = output.map((q) => q[0]).filter((v) => v > 0);
    assert(nonZeroBeforeStall.every((v) => v >= 12));

    //Deafen flushes and prevents replay on undeafen
    ctl.deafened = true;
    incoming.enqueue(new Uint8Array([0, 77]));
    await new Promise((r) => setTimeout(r, 30));
    ctl.deafened = false;
    incoming.enqueue(new Uint8Array([0, 88]));
    await waitFor(() => output.some((q) => q[0] === 176));
    assert(!output.some((q) => q[0] === 154));

    // Hold writer > 200ms while packet 99 arrives; on release, 99 must be dropped as stale.
    holdWrite = true;
    await waitFor(() => releaseWrite !== undefined);
    incoming.enqueue(new Uint8Array([0, 99]));
    await new Promise((r) => setTimeout(r, 230));
    holdWrite = false;
    releaseWrite?.();
    incoming.enqueue(new Uint8Array([0, 100]));
    await waitFor(() => output.some((q) => q[0] === 200));
    assert(!output.some((q) => q[0] === 198));
    assertEquals(maxConcurrentWrites, 1);
  } finally {
    await session.stop();
    await session.done;
  }
});

Deno.test("receive strips non-zero PLANET speech prefixes and multi-byte headers without 2.5ms CELT clipping, and send emits 0x00 then 0x10 prefixes", async () => {
  const f = fixture();
  const codec = await opusCodecFactory();
  const peerEncoder = codec.newEncoder({
    sampleRate: 48000,
    channels: 1,
    bitrate: 32000,
    frameDurationMs: 20,
    signal: "voice",
    vbr: true,
  });
  const samples = Int16Array.from(
    { length: 9600 },
    (_, i) => Math.round(6000 * Math.sin(2 * Math.PI * 300 * i / 48000)),
  );
  const f1 = peerEncoder.encode({
    samples: samples.subarray(0, 960),
    sampleRate: 48000,
    channels: 1,
  })!;
  const f2 = peerEncoder.encode({
    samples: samples.subarray(960, 1920),
    sampleRate: 48000,
    channels: 1,
  })!;
  const p1 = new Uint8Array(f1.length + 1);
  p1.set(f1, 1);
  const p2 = new Uint8Array(f2.length + 1);
  p2.set(f2, 1);
  const [paired] = packetizeNativeGroupOpusPairs([p1, p2]);
  assert(paired);

  // Check extractPlanetOpusPacket directly on non-0x00/0x10 and multi-byte prefixes
  const with80Prefix = paired.slice();
  with80Prefix[0] = 0x80; // Would have been decoded as 2.5ms (120-sample) CELT frame if unstripped!
  assertEquals(extractPlanetOpusPacket(with80Prefix).prefixOffset, 1);
  assertEquals(extractPlanetOpusPacket(with80Prefix).packet[0], 0x7b);

  const with2BytePrefix = new Uint8Array(paired.length + 1);
  with2BytePrefix[0] = 0x90;
  with2BytePrefix[1] = 0x34;
  with2BytePrefix.set(paired.subarray(1), 2);
  assertEquals(extractPlanetOpusPacket(with2BytePrefix).prefixOffset, 2);
  assertEquals(extractPlanetOpusPacket(with2BytePrefix).packet[0], 0x7b);

  const logs: string[] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => {
    if (args[0] === "[call audio stats]") logs.push(String(args[1]));
  };
  const session = await startCallAudioIO({
    transport: f.transport,
    controls,
    signal: new AbortController().signal,
    codecFactory: opusCodecFactory,
    spawnDevices: () => f.devices,
  });
  try {
    f.input().enqueue(new Uint8Array(samples.buffer));
    for (const prefix of [0x80, 0x90, 0xa0, 0x20, 0x30]) {
      const pkt = paired.slice();
      pkt[0] = prefix;
      f.incoming().enqueue(pkt);
    }
    f.incoming().enqueue(with2BytePrefix);
    await waitFor(() => f.sent.length >= 4 && f.output.length >= 12);
    assertEquals(f.sent[0][0], 0x00);
    assertEquals(f.sent[1][0], 0x00);
    assertEquals(f.sent[2][0], 0x10);
    assertEquals(f.sent[3][0], 0x10);
  } finally {
    await session.stop();
    await session.done;
    peerEncoder.close?.();
    console.error = originalError;
  }
  const stoppedLog = logs.find((l) => l.includes('"event":"stopped"'));
  assert(stoppedLog);
  const summary = JSON.parse(stoppedLog);
  assertEquals(summary.rxPackets, 6);
  assertEquals(summary.decodedFrames, 6);
  assertEquals(summary.decodedSamples, 6 * 1920);
  assertEquals(summary.rxNonStandardPrefixPackets, 6);
  assertEquals(summary.rxMultiBytePrefixPackets, 1);
  assert(summary.lastDecodedPeak < 20000);
});

Deno.test("pairedOpus mode bundles two 20ms Opus frames into 40ms Code-3 packets with timestampStep 1920", async () => {
  const sent: Array<{ payload: Uint8Array; timestampStep: number }> = [];
  let input!: ReadableStreamDefaultController<Uint8Array>;
  const devices: AudioDevices = {
    input: new ReadableStream({
      start(controller) {
        input = controller;
      },
    }),
    output: new WritableStream(),
    close: () => Promise.resolve(),
  };
  const codec = await opusCodecFactory();
  const peerDecoder = codec.newDecoder({ sampleRate: 48000, channels: 1 });
  const session = await startCallAudioIO({
    transport: {
      send(payload, options) {
        sent.push({
          payload: payload.slice(),
          timestampStep: options.timestampStep,
        });
        return Promise.resolve();
      },
      receive() {
        return {
          [Symbol.asyncIterator]: () => ({
            next: () => new Promise<IteratorResult<Uint8Array>>(() => {}),
          }),
        };
      },
    },
    controls,
    pairedOpus: true,
    signal: new AbortController().signal,
    codecFactory: opusCodecFactory,
    spawnDevices: () => devices,
  });
  try {
    const samples = Int16Array.from(
      { length: 9600 },
      (_, i) => Math.round(7000 * Math.sin(2 * Math.PI * 440 * i / 48000)),
    );
    input.enqueue(new Uint8Array(samples.buffer));
    await waitFor(() => sent.length >= 3);
    assertEquals(sent[0].timestampStep, 1920);
    assertEquals(sent[0].payload[0], 0x00);
    assertEquals(sent[1].payload[0], 0x00);
    assertEquals(sent[2].payload[0], 0x10);
    for (const pkt of sent) {
      assertEquals(pkt.payload[1] & 0x03, 0x03);
      const decoded = peerDecoder.decode(pkt.payload.subarray(1));
      assertEquals(decoded?.samples.length, 1920);
    }
  } finally {
    await session.stop();
    await session.done;
    peerDecoder.close?.();
  }
});
