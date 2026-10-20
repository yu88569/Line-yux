import { assert, assertEquals } from "@std/assert";
import { Client, TalkMessage } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { createListener, type ListenerRuntime } from "../src/listener.ts";

function clock() {
  let now = 0;
  const callbacks = new Set<() => void>();
  return {
    now: () => now,
    every(callback: () => void, _interval: number) {
      callbacks.add(callback);
      return () => {
        callbacks.delete(callback);
      };
    },
    advance(ms: number) {
      now += ms;
      for (const callback of [...callbacks]) callback();
    },
    callbacks,
  };
}

for (const healthyIdle of [false, true]) {
  Deno.test(
    healthyIdle
      ? "real idle PUSH pings keep listener healthy"
      : "listener recovers a stalled real PUSH stream exactly once",
    async () => {
      const scheduler = clock();
      let response: ReadableStreamDefaultController<Uint8Array> | undefined;
      let connects = 0;
      const base = new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: (_input: string | URL | Request, init?: RequestInit) => {
          connects++;
          const body = new ReadableStream<Uint8Array>({
            start(controller) {
              response = controller;
            },
          });
          const signal = _input instanceof Request
            ? _input.signal
            : init?.signal;
          signal?.addEventListener("abort", () => response?.close(), {
            once: true,
          });
          return Promise.resolve(new Response(body));
        },
      });
      base.authToken = "offline-test-token";
      const client = new Client(base);
      const ready = Promise.withResolvers<void>();
      const onLog = ({ type }: { type: string }) => {
        if (type === "[LEGY/PUSH] CONN start read push.") ready.resolve();
      };
      base.on("log", onLog);
      let recoveries = 0;
      let nextRead: (() => void) | undefined;
      let reads = 0;
      base.on("log", ({ type }) => {
        if (type === "[LEGY/PUSH] readByte") {
          reads++;
          nextRead?.();
        }
      });
      const runtime = {
        getClient: () => client,
        setListening: () => {},
        onTransportStalled: () => {
          recoveries++;
        },
        scheduler,
        summarizeTalkMessage: () => Promise.resolve({}),
        invalidateMessages: () => {},
        storeBoxCursor: () => {},
        upsertChatFromMessage: () => Promise.resolve(),
        hydrateLiveMedia: () => Promise.resolve(),
        upsertChatFromContact: () => Promise.resolve(),
        emitEvent: () => {},
      };
      const listener = createListener(runtime);
      try {
        listener.start();
        await ready.promise;
        if (healthyIdle) {
          for (let i = 0; i < 40; i++) {
            const read = Promise.withResolvers<void>();
            nextRead = read.resolve;
            response?.enqueue(new Uint8Array([0, 3, 1, 2, 0, 1]));
            await read.promise;
            scheduler.advance(30_000);
            assertEquals(recoveries, 0);
          }
          assertEquals(reads, 40);
          assertEquals(base.push.currPingId, 1);
          return;
        }
        scheduler.advance(119_999);
        assertEquals(recoveries, 0);
        scheduler.advance(1);
        assertEquals(recoveries, 1);
        scheduler.advance(180_000);
        assertEquals(recoveries, 1);
        assertEquals(connects, 1);
        assertEquals(base.poll.islisten, true);
      } finally {
        listener.stop();
        base.authToken = "";
        base.push.opStream.close();
        base.push.sqStream.close();
        await base.push.conns[0]?.close();
        await new Promise((resolve) => setTimeout(resolve, 4_100));
        base.off("log", onLog);
        assertEquals(base.poll.islisten, false);
      }
    },
  );
}

function fixture() {
  const scheduler = clock();
  const newClient = () => {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden");
        },
      }),
    );
    let starts = 0;
    Object.defineProperty(client, "listen", {
      value: () => {
        starts++;
      },
    });
    return { client, starts: () => starts };
  };
  const first = newClient();
  let current: Client | null = first.client;
  let recoveries = 0;
  let listening = false;
  const events: string[] = [];
  const runtime: ListenerRuntime = {
    getClient: () => current,
    setListening: (next) => {
      listening = next;
    },
    onTransportStalled: () => {
      recoveries++;
    },
    scheduler,
    summarizeTalkMessage: () => Promise.resolve({}),
    invalidateMessages: () => {
      events.push("invalidate");
    },
    storeBoxCursor: () => {
      events.push("cursor");
    },
    upsertChatFromMessage: () => {
      events.push("upsert");
      return Promise.resolve();
    },
    hydrateLiveMedia: () => {
      events.push("media");
      return Promise.resolve();
    },
    upsertChatFromContact: () => Promise.resolve(),
    emitEvent: (event) => {
      events.push(event);
    },
  };
  return {
    scheduler,
    first,
    newClient,
    runtime,
    events,
    setClient: (client: Client | null) => {
      current = client;
    },
    recoveries: () => recoveries,
    listening: () => listening,
  };
}

Deno.test("listener start and stop own one set of handlers and one timer", () => {
  const f = fixture();
  const listener = createListener(f.runtime);
  const client = f.first.client;
  try {
    listener.start();
    const staleTimer = [...f.scheduler.callbacks][0];
    listener.start();
    assertEquals(f.first.starts(), 1);
    assertEquals(f.scheduler.callbacks.size, 1);
    for (
      const name of [
        "message",
        "event",
        "call:incoming",
        "call:cancel",
      ] as const
    ) {
      assertEquals(client.listeners.get(name)?.length ?? 0, 1);
    }
    assertEquals(client.base.listeners.get("log")?.length ?? 0, 1);
    listener.stop();
    listener.stop();
    assertEquals(f.scheduler.callbacks.size, 0);
    assertEquals(f.listening(), false);
    for (
      const name of [
        "message",
        "event",
        "call:incoming",
        "call:cancel",
      ] as const
    ) {
      assertEquals(client.listeners.get(name)?.length ?? 0, 0);
    }
    assertEquals(client.base.listeners.get("log")?.length ?? 0, 0);
    staleTimer();
    f.scheduler.advance(180_000);
    assertEquals(f.recoveries(), 0);
  } finally {
    listener.stop();
  }
});

Deno.test("client replacement rejects stale progress and callbacks", () => {
  const f = fixture();
  const listener = createListener(f.runtime);
  try {
    listener.start();
    const staleTimer = [...f.scheduler.callbacks][0];
    const [staleLog] = f.first.client.base.listeners.get("log") ?? [];
    const [staleEvent] = f.first.client.listeners.get("event") ?? [];
    const next = f.newClient();
    f.setClient(next.client);
    listener.start();
    assertEquals(f.first.client.listeners.get("message")?.length ?? 0, 0);
    assertEquals(f.first.client.base.listeners.get("log")?.length ?? 0, 0);
    assertEquals(next.starts(), 1);
    assertEquals(f.scheduler.callbacks.size, 1);
    f.scheduler.advance(90_000);
    assert(staleLog);
    assert(staleEvent);
    Reflect.apply(staleLog, undefined, [{
      type: "[LEGY/PUSH] readByte",
      data: new Uint8Array([1]),
    }]);
    staleTimer();
    Reflect.apply(staleEvent, undefined, [{
      type: "NOTIFIED_READ_MESSAGE",
      param1: "old-chat",
    }]);
    assertEquals(f.events, ["listening", "listening"]);
    f.scheduler.advance(30_000);
    assertEquals(f.recoveries(), 1);
  } finally {
    listener.stop();
  }
});

Deno.test("logout without a replacement prevents watchdog recovery", () => {
  const f = fixture();
  const listener = createListener(f.runtime);
  listener.start();
  f.setClient(null);
  f.scheduler.advance(180_000);
  assertEquals(f.recoveries(), 0);
  assertEquals(f.scheduler.callbacks.size, 0);
  assertEquals(f.first.client.listeners.get("message")?.length ?? 0, 0);
  assertEquals(f.listening(), false);
  listener.stop();
});

Deno.test("only nonempty PUSH input postpones the deadline", () => {
  const f = fixture();
  const listener = createListener(f.runtime);
  const base = f.first.client.base;
  try {
    listener.start();
    f.scheduler.advance(90_000);
    base.emit("log", {
      type: "[LEGY/PUSH] readByte",
      data: new Uint8Array([1]),
    });
    f.scheduler.advance(90_000);
    assertEquals(f.recoveries(), 0);
    base.emit("log", { type: "[LEGY/PUSH] readByte", data: new Uint8Array() });
    base.emit("log", {
      type: "[LEGY/PUSH] writeByte",
      data: new Uint8Array([1]),
    });
    base.emit("log", { type: "response", data: {} });
    f.scheduler.advance(30_000);
    assertEquals(f.recoveries(), 1);
    assertEquals(f.scheduler.callbacks.size, 0);
  } finally {
    listener.stop();
  }
});

Deno.test("summary finishing after replacement cannot publish stale data", async () => {
  const f = fixture();
  const summary = Promise.withResolvers<Record<string, unknown>>();
  f.runtime.summarizeTalkMessage = () => summary.promise;
  const listener = createListener(f.runtime);
  try {
    listener.start();
    const [onMessage] = f.first.client.listeners.get("message") ?? [];
    const message = new TalkMessage({
      client: f.first.client,
      raw: {
        id: "123",
        from: "old",
        to: "peer",
        toType: "USER",
        createdTime: 1,
        deliveredTime: 1,
        text: "",
        hasContent: false,
        contentType: "NONE",
        contentPreview: "",
        contentMetadata: {},
        sessionId: 0,
        chunks: [],
        relatedMessageId: "",
        messageRelationType: 0,
        readCount: 0,
        relatedMessageServiceCode: "TALK",
        appExtensionType: 1,
        reactions: [],
        updatedTime: 1,
        location: {
          title: "",
          address: "",
          latitude: 0,
          longitude: 0,
          phone: "",
          categoryId: "",
          provider: 0,
          altitudeMeters: 0,
          accuracy: {
            radiusMeters: 0,
            radiusConfidence: 0,
            altitudeAccuracy: 0,
            velocityAccuracy: 0,
            bearingAccuracy: 0,
            accuracyMode: 0,
          },
        },
      },
    });
    assert(onMessage);
    const pending = Reflect.apply(onMessage, undefined, [message]);
    const next = f.newClient();
    f.setClient(next.client);
    listener.start();
    summary.resolve({ id: "123", from: "old", to: "peer", needsMedia: true });
    await pending;
    assertEquals(f.events, ["listening", "listening"]);
  } finally {
    listener.stop();
  }
});
