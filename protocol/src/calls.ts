import type { Client } from "@evex/linejs";
import type { LineDevice } from "./auth.ts";
import {
  type IncomingAudioCall,
  opusCodecFactory,
  parseIncomingAudioCall,
  type PlanetIncomingIdentity,
  type PlanetTransport,
  type PlanetTransportOpts,
} from "../vendor/linejs-call/entry.ts";
import { startCallAudioIO } from "./call_audio.ts";

type Json = Record<string, unknown>;
type CallTransport = Pick<
  PlanetTransport,
  | "connect"
  | "inviteDetailed"
  | "waitForAnswerDetailed"
  | "close"
  | "verifyIncomingDetailed"
  | "acceptIncomingDetailed"
  | "send"
  | "receive"
  | "ended"
>;
type CallRuntime = {
  getClient: () => Client | null;
  loadAuthDevice: () => Promise<LineDevice>;
  getIncomingIdentity: () => Promise<PlanetIncomingIdentity>;
  emitEvent: (event: string, payload?: Json) => void;
  ok: (id: number | string | null, result?: unknown) => void;
  fail: (id: number | string | null, error: string) => void;
  createTransport?: (options: PlanetTransportOpts) => CallTransport;
  codecFactory?: typeof opusCodecFactory;
  startAudio?: typeof startCallAudioIO;
};

let runtime: CallRuntime;
export function configureCallRuntime(next: CallRuntime) {
  runtime = next;
}

const LINE_CALL_DEVNAME = Deno.env.get("LINE_CALL_DEVNAME")?.trim() || "";
const LINE_CALL_DEVICE_INFO = Deno.env.get("LINE_CALL_DEVICE_INFO")?.trim() ||
  `ANDROID\t${
    Deno.env.get("LINE_VERSION")?.trim() || "26.6.2"
  }\tAndroid OS\t16`;
const signalValue = Deno.env.get("LINE_CALL_OPUS_SIGNAL")?.trim();
const opusSignal = signalValue === "auto" || signalValue === "voice"
  ? signalValue
  : "music";

type ActiveCall = {
  id: string;
  peer: string;
  abort: AbortController;
  transport?: CallTransport;
  connecting?: Promise<void>;
  audioStart?: ReturnType<typeof startCallAudioIO>;
  closing?: Promise<void>;
};
let activeCall: ActiveCall | null = null;
let incomingOffer: IncomingAudioCall | null = null;
const seenIncoming = new Map<string, number>();
function expireIncoming() {
  if (incomingOffer && !offerFresh(incomingOffer)) incomingOffer = null;
  for (const [id, expires] of seenIncoming) {
    if (expires < Date.now()) seenIncoming.delete(id);
  }
}
let callAudioInput = Deno.env.get("LINE_GTK_AUDIO_INPUT")?.trim() || "default";
let callAudioOutput = Deno.env.get("LINE_GTK_AUDIO_OUTPUT")?.trim() ||
  "default";
const callAudioCtl = { muted: false, deafened: false, micGain: 1, spkGain: 1 };

export function setCallAudioDevices(input?: string, output?: string) {
  if (input?.trim()) callAudioInput = input.trim();
  if (output?.trim()) callAudioOutput = output.trim();
}
function clampGain(value: unknown, fallback: number): number {
  const gain = typeof value === "number" ? value : Number(value);
  return Number.isFinite(gain) ? Math.min(2.5, Math.max(0, gain)) : fallback;
}
export function setCallGains(micGain?: unknown, spkGain?: unknown) {
  if (micGain !== undefined) {
    callAudioCtl.micGain = clampGain(micGain, callAudioCtl.micGain);
  }
  if (spkGain !== undefined) {
    callAudioCtl.spkGain = clampGain(spkGain, callAudioCtl.spkGain);
  }
}
export function updateCallAudio(params: Json) {
  if (params.muted !== undefined) callAudioCtl.muted = !!params.muted;
  if (params.deafened !== undefined) callAudioCtl.deafened = !!params.deafened;
  setCallGains(params.micGain, params.spkGain);
  return { ...callAudioCtl };
}
function state(call: ActiveCall, value: string, error?: string) {
  runtime.emitEvent("call_state", {
    callId: call.id,
    peer: call.peer,
    state: value,
    ...(error ? { error } : {}),
  });
}
function current(call: ActiveCall) {
  return activeCall === call && !call.abort.signal.aborted;
}
function reserve(id: string, peer: string): ActiveCall {
  const call = { id, peer, abort: new AbortController() };
  activeCall = call;
  callAudioCtl.muted = false;
  callAudioCtl.deafened = false;
  return call;
}
function endCall(call: ActiveCall, silent = false): Promise<void> {
  if (call.closing) return call.closing;
  call.abort.abort();
  if (!silent && activeCall === call) state(call, "ended");
  call.closing = (async () => {
    await call.connecting?.catch(() => {});
    await call.transport?.close().catch(() => {});
    const audio = await call.audioStart?.catch(() => undefined);
    await audio?.stop().catch(() => {});
    if (activeCall === call) activeCall = null;
  })();
  return call.closing;
}
function failCall(call: ActiveCall, error: unknown) {
  if (!current(call)) return;
  state(call, "failed", error instanceof Error ? error.message : String(error));
  return endCall(call, true);
}
async function makeTransport(call: ActiveCall, localMid: string) {
  const options: PlanetTransportOpts = {
    localMid,
    timeoutMs: 10_000,
    mediaKeyMode: "audio-reverse-stage",
    deviceInfo: LINE_CALL_DEVICE_INFO,
  };
  const transport = runtime.createTransport
    ? runtime.createTransport(options)
    : new (await import("../vendor/linejs-call/entry.ts")).PlanetTransport(
      options,
    );
  if (!current(call)) {
    await transport.close().catch(() => {});
    return;
  }
  call.transport = transport;
  return transport;
}
function watchTransport(call: ActiveCall, transport: CallTransport) {
  void transport.ended.then((reason) => {
    if (current(call) && reason.by === "remote") void endCall(call);
  }).catch((error) => {
    void failCall(call, error);
  });
}
async function startAudio(call: ActiveCall, transport: CallTransport) {
  if (!current(call)) return;
  call.audioStart = (runtime.startAudio ?? startCallAudioIO)({
    transport,
    codecFactory: runtime.codecFactory ?? opusCodecFactory,
    signal: call.abort.signal,
    controls: callAudioCtl,
    inputDevice: callAudioInput,
    outputDevice: callAudioOutput,
    opusSignal,
    onEnded: (error) => {
      if (!current(call)) return;
      if (error) void failCall(call, error);
      else void endCall(call);
    },
  });
  await call.audioStart;
  if (!current(call)) return;
  state(call, "connected");
}

export async function doCallStart(id: number | string | null, peerMid: string) {
  const client = runtime.getClient();
  if (!client) {
    runtime.fail(id, "not_logged_in");
    return;
  }
  if (!/^u[0-9a-f]{32}$/.test(peerMid)) {
    runtime.fail(id, "voice_call_dm_only");
    return;
  }
  expireIncoming();
  if (activeCall || incomingOffer) {
    runtime.fail(id, "call_already_active");
    return;
  }
  const call = reserve(`out-${crypto.randomUUID()}`, peerMid);
  try {
    if (await runtime.loadAuthDevice() === "DESKTOPWIN") {
      if (current(call)) runtime.fail(id, "relogin_android_required");
      await endCall(call, true);
      return;
    }
    if (!current(call)) return;
    const localMid = client.base.profile?.mid;
    if (!localMid) throw new Error("profile not ready");
    runtime.ok(id, { callId: call.id, peer: peerMid, state: "starting" });
    state(call, "acquiring");
    const route = await client.call.acquireRoute({
      to: peerMid,
      callType: "AUDIO",
      ...(LINE_CALL_DEVNAME
        ? { fromEnvInfo: { devname: LINE_CALL_DEVNAME } }
        : {}),
    });
    if (!current(call)) return;
    if (route.fakeCall) {
      throw new Error("LINE returned fakeCall=true; peer will not ring");
    }
    const transport = await makeTransport(call, localMid);
    if (!transport) return;
    await (call.connecting = transport.connect({ route }));
    if (!current(call)) return;
    watchTransport(call, transport);
    await transport.inviteDetailed({ to: peerMid });
    if (!current(call)) return;
    state(call, "ringing");
    const answer = await transport.waitForAnswerDetailed({
      timeoutMs: 60_000,
      autoConnRsp: true,
    });
    if (!current(call)) return;
    if (!answer.mediaReady) throw new Error("answered without media");
    await startAudio(call, transport);
  } catch (error) {
    await failCall(call, error);
  }
}

async function connectIncoming(call: ActiveCall, offer: IncomingAudioCall) {
  const identity = await runtime.getIncomingIdentity();
  if (!current(call)) return;
  const transport = await makeTransport(call, offer.localMid);
  if (!transport) return;
  await (call.connecting = transport.connect({ route: offer.route }));
  if (!current(call)) return;
  watchTransport(call, transport);
  await transport.verifyIncomingDetailed(offer, identity, call.abort.signal);
  return current(call) ? transport : undefined;
}
function offerFresh(offer: IncomingAudioCall) {
  const age = Date.now() - offer.createdAt;
  return age >= -30_000 && age <= 120_000;
}
export async function doCallAnswer(id: number | string | null) {
  if (!runtime.getClient()) {
    runtime.fail(id, "not_logged_in");
    return;
  }
  if (activeCall) {
    runtime.fail(id, "call_already_active");
    return;
  }
  const offer = incomingOffer;
  incomingOffer = null;
  if (!offer || !offerFresh(offer)) {
    runtime.fail(id, "no_incoming_call");
    return;
  }
  const call = reserve(offer.callId, offer.callerMid);
  runtime.ok(id, { answering: true, peer: call.peer, callId: call.id });
  state(call, "answering");
  try {
    const transport = await connectIncoming(call, offer);
    if (!transport) return;
    await transport.acceptIncomingDetailed();
    if (current(call)) await startAudio(call, transport);
  } catch (error) {
    await failCall(call, error);
  }
}
export async function doCallDecline(id: number | string | null) {
  if (activeCall) {
    runtime.fail(id, "call_already_active");
    return;
  }
  const offer = incomingOffer;
  incomingOffer = null;
  if (offer && offerFresh(offer) && runtime.getClient()) {
    const call = reserve(offer.callId, offer.callerMid);
    try {
      await connectIncoming(call, offer);
    } catch (error) {
      await failCall(call, error);
    } finally {
      await endCall(call);
    }
  }
  runtime.ok(id, { declined: true });
}
export async function doCallEnd(id: number | string | null) {
  const offer = incomingOffer;
  incomingOffer = null;
  if (activeCall) await endCall(activeCall);
  else if (offer) {
    runtime.emitEvent("call_state", {
      callId: offer.callId,
      peer: offer.callerMid,
      state: "ended",
    });
  }
  runtime.ok(id, { ended: true });
}
export async function shutdownCalls(): Promise<void> {
  incomingOffer = null;
  if (activeCall) await endCall(activeCall, true);
}
export function handleIncomingCall(
  ev: { raw?: unknown; callMid?: string; from?: string; kind?: string },
) {
  expireIncoming();
  if (activeCall || incomingOffer) return;
  const localMid = runtime.getClient()?.base.profile?.mid;
  if (!localMid) return;
  const offer = parseIncomingAudioCall(ev.raw, localMid);
  if (!offer || !offerFresh(offer) || seenIncoming.has(offer.callId)) return;
  seenIncoming.set(offer.callId, offer.createdAt + 120_000);
  if (seenIncoming.size > 256) {
    seenIncoming.delete(seenIncoming.keys().next().value ?? "");
  }
  incomingOffer = offer;
  runtime.emitEvent("call_incoming", {
    callId: offer.callId,
    from: offer.callerMid,
    kind: "AUDIO",
  });
}
export function handleCallCancel(
  ev: { raw?: unknown; callMid?: string; from?: string; reason?: string },
) {
  // The cached library maps cancellation param1 to callMid. Never fall back to
  // an empty sender or a peer match: a delayed push could end their next call.
  const callId = ev.callMid;
  if (
    !callId || (callId !== incomingOffer?.callId && callId !== activeCall?.id)
  ) return;
  const from = incomingOffer?.callerMid ?? activeCall?.peer ?? "";
  if (incomingOffer?.callId === callId) incomingOffer = null;
  runtime.emitEvent("call_canceled", { callId, from, reason: ev.reason ?? "" });
  if (activeCall?.id === callId) void endCall(activeCall);
}
