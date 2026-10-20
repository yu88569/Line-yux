import { assert, assertEquals } from "@std/assert";
import { Client } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import {
  configureCallRuntime,
  doCallAnswer,
  doCallDecline,
  doCallEnd,
  doCallStart,
  handleCallCancel,
  handleIncomingCall,
  shutdownCalls,
} from "../src/calls.ts";
import type { PlanetEndReason } from "../vendor/linejs-call/entry.ts";

type Runtime = Parameters<typeof configureCallRuntime>[0];
const caller = `u${"1".repeat(32)}`;
const local = `u${"2".repeat(32)}`;
let nextCall = 0;
function rawOffer(callId: string, patch: Record<string, unknown> = {}) {
  return {
    type: "NOTIFIED_RECEIVED_CALL",
    param1: caller,
    param2: "token",
    createdTime: Date.now(),
    param3: JSON.stringify({
      k: "CA",
      n: "token",
      vs: callId,
      vfz: "JP",
      vtz: "JP",
      h: "127.0.0.1",
      p: 9000,
      vc: JSON.stringify({
        mpkey: btoa(String.fromCharCode(...new Uint8Array(33).fill(2))),
      }),
      ...patch,
    }),
  };
}
function harness(
  options: {
    startAudio?: Runtime["startAudio"];
    connect?: () => Promise<void>;
    loadAuthDevice?: Runtime["loadAuthDevice"];
  } = {},
) {
  const client = new Client(
    new BaseClient({
      device: "ANDROIDSECONDARY",
      fetch: () => {
        throw new Error("network forbidden");
      },
    }),
  );
  Object.defineProperty(client.base, "profile", { value: { mid: local } });
  let acquired = 0;
  Object.defineProperty(client.call, "acquireRoute", {
    value: () => {
      acquired++;
      throw new Error("outgoing route forbidden");
    },
  });
  const actions: string[] = [];
  const events: { event: string; payload?: Record<string, unknown> }[] = [];
  const errors: string[] = [];
  let transportOptions:
    | Parameters<NonNullable<Runtime["createTransport"]>>[0]
    | undefined;
  const ended = Promise.withResolvers<PlanetEndReason>();
  const identity = {
    deviceId: "test-installation",
    userAgent: { osName: "ANDROID", osVersion: "16", deviceName: "test" },
  };
  const transport: ReturnType<NonNullable<Runtime["createTransport"]>> = {
    ended: ended.promise,
    async connect({ route }) {
      actions.push("connect");
      assert("toMid" in route);
      assertEquals(route.toMid, caller);
      await options.connect?.();
    },
    verifyIncomingDetailed(call, receivedIdentity, signal) {
      actions.push("verify");
      assertEquals(call.localMid, local);
      assertEquals(call.callerMid, caller);
      assertEquals(receivedIdentity, identity);
      assert(signal);
      assertEquals(signal.aborted, false);
      return Promise.resolve();
    },
    acceptIncomingDetailed() {
      actions.push("accept");
      return Promise.resolve({ mediaReady: true });
    },
    inviteDetailed() {
      throw new Error("outgoing invite forbidden");
    },
    waitForAnswerDetailed() {
      throw new Error("outgoing answer wait forbidden");
    },
    close() {
      actions.push("close");
      ended.resolve({ by: "local" });
      return Promise.resolve();
    },
    send() {
      return Promise.resolve();
    },
    async *receive() {
      yield new Uint8Array();
    },
  };
  configureCallRuntime({
    getClient: () => client,
    loadAuthDevice: options.loadAuthDevice ??
      (() => Promise.resolve("ANDROIDSECONDARY")),
    getIncomingIdentity: () => Promise.resolve(identity),
    createTransport: (options) => {
      transportOptions = options;
      return transport;
    },
    startAudio: options.startAudio ?? (() => {
      actions.push("audio");
      return Promise.resolve({
        stop: () => {
          actions.push("audio-stop");
          return Promise.resolve();
        },
        done: Promise.resolve(),
      });
    }),
    emitEvent: (event, payload) => events.push({ event, payload }),
    ok: () => {},
    fail: (_id, error) => errors.push(error),
  });
  const callId = `incoming-${++nextCall}`;
  const offer = rawOffer(callId);
  const incoming = () =>
    handleIncomingCall({ callMid: caller, from: "token", raw: offer });
  return {
    callId,
    offer,
    incoming,
    actions,
    events,
    errors,
    ended,
    get transportOptions() {
      return transportOptions;
    },
    get acquired() {
      return acquired;
    },
  };
}

Deno.test("call receive diagnostics summarize allowlisted health without packet or identity data", async () => {
  const logs: string[] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => logs.push(args.join(" "));
  const h = harness();
  try {
    h.incoming();
    await doCallAnswer(1);
    const debug = h.transportOptions?.debug;
    assert(debug);
    const secret = "do-not-log-secret";
    debug({ type: "media_key_selected", send: secret, recv: secret });
    for (let i = 0; i < 1000; i++) {
      debug({ type: "recv", sourcePort: secret, sourceFamily: secret });
      debug({ type: "rtp_recv", ssrc: secret, seq: i });
      debug({ type: "media_recv", payloadType: 96, payload: secret });
    }
    debug({ type: "media_recv", payloadType: secret });
    debug({ type: "media_decrypt_fail", reason: secret });
    debug({ type: "media_decrypt_fail", reason: "SRTP auth tag mismatch" });
    await doCallEnd(null);
    const combined = logs.join("\n");
    assert(!combined.includes(secret));
    assert(!combined.includes(local));
    assert(!combined.includes(caller));
    assert(logs.length < 20, "receive diagnostics must aggregate packet logs");
    assert(logs[0].includes('"phase":"monitoring"'));
    const final = logs.at(-1);
    assert(final);
    assert(final.includes('"phase":"closed"'));
    assert(final.includes('"udpPackets":1000'));
    assert(final.includes('"srtpPackets":1000'));
    assert(final.includes('"authenticatedPackets":1001'));
    assert(final.includes('"authenticatedByPayloadType":{"96":1000}'));
    assert(final.includes('"decryptFailures":2'));
    assert(final.includes('"firstFailure":"other"'));
  } finally {
    await shutdownCalls();
    console.error = originalError;
  }
});

Deno.test("call receive diagnostics report zero SRTP at closure", async () => {
  const logs: string[] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => logs.push(args.join(" "));
  const h = harness();
  try {
    h.incoming();
    await doCallAnswer(1);
    await doCallEnd(null);
    const final = logs.at(-1);
    assert(final);
    assert(final.includes('"srtpPackets":0'));
    assert(final.includes('"authenticatedPackets":0'));
    assert(final.includes('"firstFailure":"none"'));
  } finally {
    await shutdownCalls();
    console.error = originalError;
  }
});

Deno.test("call signaling diagnostics retain numeric setup status and omit phrases and session data", async () => {
  const logs: string[] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => logs.push(args.join(" "));
  const h = harness();
  try {
    h.incoming();
    await doCallAnswer(1);
    const debug = h.transportOptions?.debug;
    assert(debug);
    const secret = "private-phrase-session-key";
    for (let i = 0; i < 1000; i++) {
      debug({ type: "decrypt_ok", plaintext: secret });
      debug({ type: "decrypt_fail", keys: secret });
      debug({ type: "planet_msg", cc: "alive_rpt", session: secret });
    }
    debug({ type: "planet_msg", cc: secret, header: secret });
    const beforeSetup = logs.length;
    debug({
      type: "setup_rsp",
      result: 1,
      relCode: 600,
      relPhrase: secret,
      session: secret,
    });
    assertEquals(
      logs.length,
      beforeSetup + 1,
      "first setup response must log immediately",
    );
    debug({
      type: "rel_req",
      relCode: 601,
      relPhrase: secret,
      releaser: secret,
    });
    debug({ type: "rel_req", relCode: secret });
    await doCallEnd(null);
    assert(!logs.join("\n").includes(secret));
    assert(logs.length < 20);
    const final = logs.at(-1);
    assert(final);
    assert(final.includes('"signalingDecryptOk":1000'));
    assert(final.includes('"signalingDecryptFailures":1000'));
    assert(final.includes('"ccMessages":{"alive_rpt":1000,"other":1}'));
    assert(final.includes('"firstSetup":{"result":1,"relCode":600}'));
    assert(final.includes('"releaseRequests":2'));
    assert(final.includes('"releaseCodes":{"601":1}'));
  } finally {
    await shutdownCalls();
    console.error = originalError;
  }
});

Deno.test("answering native incoming audio accepts its route and identity without an outgoing call", async () => {
  const h = harness();
  try {
    h.incoming();
    await doCallAnswer(1);
    assertEquals(h.acquired, 0);
    assertEquals(h.actions, ["connect", "verify", "accept", "audio"]);
    assertEquals(h.events[0], {
      event: "call_incoming",
      payload: { callId: h.callId, from: caller, kind: "AUDIO" },
    });
    assertEquals(h.events.at(-1)?.payload?.state, "connected");
    await doCallEnd(null);
    assertEquals(h.actions.filter((a) => a === "close").length, 1);
  } finally {
    await shutdownCalls();
  }
});
Deno.test("decline verifies and releases the inbound route without accepting or calling back", async () => {
  const h = harness();
  try {
    h.incoming();
    await doCallDecline(1);
    assertEquals(h.acquired, 0);
    assertEquals(h.actions, ["connect", "verify", "close"]);
    assertEquals(h.events.at(-1)?.payload?.state, "ended");
    h.incoming();
    assertEquals(
      h.events.filter((ev) => ev.event === "call_incoming").length,
      1,
    );
  } finally {
    await shutdownCalls();
  }
});
Deno.test("incoming rejects stale video group offers and preserves a busy or duplicate offer", async () => {
  const h = harness();
  try {
    handleIncomingCall({
      raw: { ...h.offer, createdTime: Date.now() - 120_001 },
    });
    handleIncomingCall({ raw: rawOffer(h.callId, { k: "CV" }) });
    handleIncomingCall({ raw: rawOffer(h.callId, { g: "group" }) });
    handleIncomingCall({ callMid: "legacy", from: caller });
    assertEquals(h.events.length, 0);
    h.incoming();
    h.incoming();
    handleIncomingCall({ raw: rawOffer("unrelated-busy") });
    assertEquals(h.events.length, 1);
    await doCallAnswer(1);
    handleIncomingCall({ raw: rawOffer("unrelated-active") });
    assertEquals(
      h.events.filter((ev) => ev.event === "call_incoming").length,
      1,
    );
  } finally {
    await shutdownCalls();
  }
});
Deno.test("empty or unrelated cancel never ends an incoming or active call", async () => {
  const h = harness();
  try {
    h.incoming();
    handleCallCancel({});
    handleCallCancel({ callMid: "other", from: caller });
    await doCallAnswer(1);
    handleCallCancel({ callMid: "other", from: caller });
    assertEquals(h.actions.includes("close"), false);
    handleCallCancel({ callMid: h.callId, from: caller });
    await shutdownCalls();
    assertEquals(h.actions.filter((a) => a === "close").length, 1);
  } finally {
    await shutdownCalls();
  }
});
Deno.test("hangup during audio startup joins cleanup and cannot emit late connected", async () => {
  const started = Promise.withResolvers<void>();
  const audio = Promise.withResolvers<
    Awaited<ReturnType<NonNullable<Runtime["startAudio"]>>>
  >();
  let stops = 0;
  const h = harness({
    startAudio: () => {
      started.resolve();
      return audio.promise;
    },
  });
  try {
    h.incoming();
    const answer = doCallAnswer(1);
    await started.promise;
    const hangup = doCallEnd(2);
    audio.resolve({
      stop: () => {
        stops++;
        return Promise.resolve();
      },
      done: Promise.resolve(),
    });
    await Promise.all([answer, hangup]);
    assertEquals(
      h.events.some((ev) => ev.payload?.state === "connected"),
      false,
    );
    assertEquals(h.actions.filter((a) => a === "close").length, 1);
    assertEquals(stops, 1);
  } finally {
    await shutdownCalls();
  }
});
Deno.test("hangup during connect waits for pending bind then closes transport", async () => {
  const connecting = Promise.withResolvers<void>();
  const connected = Promise.withResolvers<void>();
  const h = harness({
    connect: () => {
      connecting.resolve();
      return connected.promise;
    },
  });
  try {
    h.incoming();
    const answer = doCallAnswer(1);
    await connecting.promise;
    const hangup = doCallEnd(2);
    assertEquals(h.actions.includes("close"), false);
    connected.resolve();
    await Promise.all([answer, hangup]);
    assertEquals(h.actions, ["connect", "close"]);
    assertEquals(
      h.events.some((ev) => ev.payload?.state === "connected"),
      false,
    );
  } finally {
    await shutdownCalls();
  }
});
Deno.test("audio startup failure closes transport and emits failed before connected", async () => {
  const h = harness({
    startAudio: () => Promise.reject(new Error("microphone failed")),
  });
  try {
    h.incoming();
    await doCallAnswer(1);
    assertEquals(h.events.at(-1)?.payload?.state, "failed");
    assertEquals(h.events.at(-1)?.payload?.error, "microphone failed");
    assertEquals(
      h.events.some((ev) => ev.payload?.state === "connected"),
      false,
    );
    assertEquals(h.actions.filter((a) => a === "close").length, 1);
  } finally {
    await shutdownCalls();
  }
});
Deno.test("outgoing reserves before device lookup and hangup prevents late route acquisition", async () => {
  const device = Promise.withResolvers<"ANDROIDSECONDARY">();
  const h = harness({ loadAuthDevice: () => device.promise });
  try {
    const first = doCallStart(1, caller);
    await doCallStart(2, caller);
    assertEquals(h.errors, ["call_already_active"]);
    await doCallEnd(3);
    device.resolve("ANDROIDSECONDARY");
    await first;
    assertEquals(h.acquired, 0);
  } finally {
    await shutdownCalls();
  }
});
Deno.test("remote release ends and cleans up exactly its active session", async () => {
  const h = harness();
  try {
    h.incoming();
    await doCallAnswer(1);
    h.ended.resolve({ by: "remote", relCode: 0 });
    await Promise.resolve();
    await shutdownCalls();
    assertEquals(h.events.at(-1)?.payload?.state, "ended");
    assertEquals(h.actions.filter((a) => a === "close").length, 1);
  } finally {
    await shutdownCalls();
  }
});
