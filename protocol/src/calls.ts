import type { Client } from "@evex/linejs";
import type { LineDevice } from "./auth.ts";
import {
  type IncomingCall,
  opusCodecFactory,
  parseIncomingCall,
  type PlanetIncomingIdentity,
  type PlanetTransport,
  type PlanetTransportOpts,
} from "../vendor/linejs-call/entry.ts";
import { startCallAudioIO } from "./call_audio.ts";
import { encodeJpegBase64, startCallVideoIO } from "./call_video.ts";
import {
  CC_MSG_NAMES,
  MC_MSG_NAMES,
} from "../vendor/linejs-call/planet/schema.ts";

type Json = Record<string, unknown>;
type CallKind = "AUDIO" | "VIDEO";
type CallTransport =
  & Pick<
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
  >
  & Partial<Pick<PlanetTransport, "sendVideo" | "receiveVideo">>;
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
  startVideo?: typeof startCallVideoIO;
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
const opusSignal = signalValue === "auto" || signalValue === "music"
  ? signalValue
  : "voice";

type ActiveCall = {
  id: string;
  peer: string;
  kind: CallKind;
  abort: AbortController;
  transport?: CallTransport;
  connecting?: Promise<void>;
  audioStart?: ReturnType<typeof startCallAudioIO>;
  videoSession?: ReturnType<typeof startCallVideoIO>;
  closing?: Promise<void>;
  logMediaSummary?: () => void;
};
let activeCall: ActiveCall | null = null;
let incomingOffer: IncomingCall | null = null;
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
let callVideoInput = Deno.env.get("LINE_GTK_VIDEO_INPUT")?.trim() || "";
const callAudioCtl = { muted: false, deafened: false, micGain: 1, spkGain: 1 };
const callVideoCtl = { cameraEnabled: true, mirror: true };

export function setCallAudioDevices(input?: string, output?: string) {
  if (input?.trim()) callAudioInput = input.trim();
  if (output?.trim()) callAudioOutput = output.trim();
}
export function setCallVideoDevice(videoInput?: string, mirror?: unknown) {
  if (typeof videoInput === "string") callVideoInput = videoInput.trim();
  if (typeof mirror === "boolean") callVideoCtl.mirror = mirror;
}
function clampGain(value: unknown, fallback: number): number {
  const gain = typeof value === "number" ? value : Number(value);
  return Number.isFinite(gain) ? Math.min(4, Math.max(0, gain)) : fallback;
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
export function updateCallVideo(params: Json) {
  if (params.cameraEnabled !== undefined) {
    callVideoCtl.cameraEnabled = !!params.cameraEnabled;
  }
  if (params.mirror !== undefined) {
    callVideoCtl.mirror = !!params.mirror;
  }
  return { ...callVideoCtl };
}
function state(call: ActiveCall, value: string, error?: string) {
  runtime.emitEvent("call_state", {
    callId: call.id,
    peer: call.peer,
    kind: call.kind,
    state: value,
    ...(error ? { error } : {}),
  });
}
function current(call: ActiveCall) {
  return activeCall === call && !call.abort.signal.aborted;
}
function reserve(
  id: string,
  peer: string,
  kind: CallKind = "AUDIO",
): ActiveCall {
  const call: ActiveCall = { id, peer, kind, abort: new AbortController() };
  activeCall = call;
  callAudioCtl.muted = false;
  callAudioCtl.deafened = false;
  callVideoCtl.cameraEnabled = kind === "VIDEO";
  return call;
}
function endCall(call: ActiveCall, silent = false): Promise<void> {
  if (call.closing) return call.closing;
  call.abort.abort();
  if (!silent && activeCall === call) state(call, "ended");
  call.closing = (async () => {
    await call.connecting?.catch(() => {});
    await call.transport?.close().catch(() => {});
    call.logMediaSummary?.();
    const audio = await call.audioStart?.catch(() => undefined);
    await audio?.stop().catch(() => {});
    await call.videoSession?.stop().catch(() => {});
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
  const received = {
    udpPackets: 0,
    srtpPackets: 0,
    authenticatedPackets: 0,
    decryptFailures: 0,
    authenticatedByPayloadType: new Map<number, number>(),
    firstFailure: "none",
    signalingDecryptOk: 0,
    signalingDecryptFailures: 0,
    ccMessages: new Map<string, number>(),
    mcMessages: new Map<string, number>(),
    setupResponses: 0,
    releaseRequests: 0,
    releaseCodes: new Map<number, number>(),
    txMediaPackets: 0,
    txMediaBytes: 0,
    txSsrc: null as number | null,
    txPayloadType: null as number | null,
    txRtpFirstByte: null as number | null,
    dataRtpSent: 0,
    direction: "unknown",
    endpointSource: "none",
    endpointLearned: 0,
    endpointLearnSkipped: 0,
    peerHasMediaPubKey: null as boolean | null,
    peerHasMediaSecret: null as boolean | null,
    peerMediaKeyIdIsZero: null as boolean | null,
    peerMediaKeyIdBits: null as number | null,
    peerMedia: [] as Array<Record<string, string | number | null>>,
    mcDataReqSpec: null as Record<string, unknown> | null,
    dataSendMode: "none",
    rxSsrc: null as number | null,
    rxAudioSsrc: null as number | null,
    rxAudioRtpFirstByte: null as number | null,
    rxAudioMarkerCount: 0,
    rxAudioExtProfile: null as number | null,
    rxAudioExtWords: null as number | null,
    rxAudioExtSamples: [] as string[],
    rxAudioTsDeltas: [] as number[],
    rxNonAudioSamples: [] as string[],
    decryptFailSamples: [] as string[],
    rxMediaModes: new Map<string, number>(),
  };
  let lastRxAudioTs: number | null = null;
  const endpointSources = new Set([
    "mAddr",
    "uePublicAddr",
    "route",
    "session",
  ]);
  const dataSendModes = new Set(["none", "secret", "ecdh-reverse-stage"]);
  const mediaModePattern = /^[a-z0-9-]{1,40}$/;
  const hexPattern = /^[0-9a-f]{2,24}$/;
  let firstSetup: { result: number | null; relCode: number | null } | null =
    null;
  const ccNames = new Set(Object.values(CC_MSG_NAMES));
  const mcNames = new Set(Object.values(MC_MSG_NAMES));
  const logSummary = (phase: "monitoring" | "receiving" | "closed") => {
    console.error(
      "[call media]",
      JSON.stringify({
        phase,
        ...received,
        authenticatedByPayloadType: Object.fromEntries(
          received.authenticatedByPayloadType,
        ),
        ccMessages: Object.fromEntries(received.ccMessages),
        mcMessages: Object.fromEntries(received.mcMessages),
        rxMediaModes: Object.fromEntries(received.rxMediaModes),
        firstSetup,
        releaseCodes: Object.fromEntries(received.releaseCodes),
      }),
    );
  };
  const failureReasons = new Set([
    "SRTP auth tag mismatch",
    "SRTP packet too short",
    "RTP too short",
    "RTP CSRC header truncated",
    "RTP extension header truncated",
    "RTP extension data truncated",
  ]);
  let nextSummaryAt = 0;
  call.logMediaSummary = () => logSummary("closed");
  const options: PlanetTransportOpts = {
    localMid,
    timeoutMs: 10_000,
    mediaKeyMode: "audio-reverse-stage",
    callType: call.kind,
    deviceInfo: LINE_CALL_DEVICE_INFO,
    debug(event) {
      let first = false;
      switch (event.type) {
        case "decrypt_ok":
          received.signalingDecryptOk++;
          first = received.signalingDecryptOk === 1;
          break;
        case "decrypt_fail":
          received.signalingDecryptFailures++;
          first = received.signalingDecryptFailures === 1;
          break;
        case "planet_msg": {
          const name = typeof event.cc === "string" && ccNames.has(event.cc)
            ? event.cc
            : "other";
          const count = received.ccMessages.get(name) ?? 0;
          received.ccMessages.set(name, count + 1);
          first = count === 0;
          if (typeof event.mc === "string" && mcNames.has(event.mc)) {
            const mcCount = received.mcMessages.get(event.mc) ?? 0;
            received.mcMessages.set(event.mc, mcCount + 1);
            if (mcCount === 0) first = true;
          }
          break;
        }
        case "setup_rsp":
          received.setupResponses++;
          first = received.setupResponses === 1;
          if (first) {
            firstSetup = {
              result: typeof event.result === "number" &&
                  Number.isSafeInteger(event.result)
                ? event.result
                : null,
              relCode: typeof event.relCode === "number" &&
                  Number.isSafeInteger(event.relCode)
                ? event.relCode
                : null,
            };
          }
          break;
        case "rel_req": {
          received.releaseRequests++;
          first = received.releaseRequests === 1;
          const code = event.relCode;
          if (typeof code === "number" && Number.isSafeInteger(code)) {
            const count = received.releaseCodes.get(code) ?? 0;
            received.releaseCodes.set(code, count + 1);
          }
          break;
        }
        case "recv":
          received.udpPackets++;
          first = received.udpPackets === 1;
          break;
        case "rtp_recv":
          received.srtpPackets++;
          first = received.srtpPackets === 1;
          break;
        case "media_recv": {
          received.authenticatedPackets++;
          first = received.authenticatedPackets === 1;
          const pt = event.payloadType;
          if (
            typeof pt === "number" && Number.isInteger(pt) && pt >= 0 &&
            pt <= 127
          ) {
            const count = received.authenticatedByPayloadType.get(pt) ?? 0;
            received.authenticatedByPayloadType.set(pt, count + 1);
          }
          if (
            typeof event.mediaKeyMode === "string" &&
            mediaModePattern.test(event.mediaKeyMode) &&
            (received.rxMediaModes.has(event.mediaKeyMode) ||
              received.rxMediaModes.size < 8)
          ) {
            received.rxMediaModes.set(
              event.mediaKeyMode,
              (received.rxMediaModes.get(event.mediaKeyMode) ?? 0) + 1,
            );
          }
          if (
            received.rxSsrc === null && typeof event.ssrc === "number" &&
            Number.isSafeInteger(event.ssrc) && event.ssrc >= 0
          ) {
            received.rxSsrc = event.ssrc;
          }
          const audioPt = received.txPayloadType ?? 96;
          if (pt === audioPt) {
            if (
              received.rxAudioSsrc === null && typeof event.ssrc === "number" &&
              Number.isSafeInteger(event.ssrc) && event.ssrc >= 0
            ) {
              received.rxAudioSsrc = event.ssrc;
            }
            if (
              received.rxAudioRtpFirstByte === null &&
              typeof event.rtpFirstByte === "number" &&
              Number.isInteger(event.rtpFirstByte) && event.rtpFirstByte >= 0 &&
              event.rtpFirstByte <= 255
            ) {
              received.rxAudioRtpFirstByte = event.rtpFirstByte;
            }
            if (event.marker === true) received.rxAudioMarkerCount++;
            if (
              received.rxAudioExtProfile === null &&
              typeof event.extProfile === "number" &&
              Number.isInteger(event.extProfile) && event.extProfile >= 0 &&
              event.extProfile <= 0xffff
            ) {
              received.rxAudioExtProfile = event.extProfile;
            }
            if (
              received.rxAudioExtWords === null &&
              typeof event.extWords === "number" &&
              Number.isInteger(event.extWords) && event.extWords >= 0 &&
              event.extWords <= 1024
            ) {
              received.rxAudioExtWords = event.extWords;
            }
            if (
              typeof event.extHeadHex === "string" &&
              hexPattern.test(event.extHeadHex) &&
              received.rxAudioExtSamples.length < 4 &&
              !received.rxAudioExtSamples.includes(event.extHeadHex)
            ) {
              received.rxAudioExtSamples.push(event.extHeadHex);
            }
            if (
              typeof event.timestamp === "number" &&
              Number.isSafeInteger(event.timestamp) && event.timestamp >= 0
            ) {
              if (
                lastRxAudioTs !== null && received.rxAudioTsDeltas.length < 6
              ) {
                received.rxAudioTsDeltas.push(
                  (event.timestamp - lastRxAudioTs) >>> 0,
                );
              }
              lastRxAudioTs = event.timestamp;
            }
          } else if (
            typeof event.nonAudioHeadHex === "string" &&
            hexPattern.test(event.nonAudioHeadHex) &&
            received.rxNonAudioSamples.length < 4
          ) {
            const pLen = typeof event.payloadBytes === "number" &&
                Number.isSafeInteger(event.payloadBytes)
              ? event.payloadBytes
              : -1;
            const ssrcNum = typeof event.ssrc === "number" &&
                Number.isSafeInteger(event.ssrc)
              ? event.ssrc
              : -1;
            const sample = `${
              pt ?? -1
            }|${ssrcNum}|${pLen}|${event.nonAudioHeadHex}`;
            if (!received.rxNonAudioSamples.includes(sample)) {
              received.rxNonAudioSamples.push(sample);
            }
          }
          break;
        }
        case "media_configured": {
          first = received.endpointSource === "none";
          if (
            event.direction === "incoming" || event.direction === "outgoing"
          ) {
            received.direction = event.direction;
          }
          received.endpointSource = typeof event.endpoint === "string" &&
              endpointSources.has(event.endpoint)
            ? event.endpoint
            : "other";
          if (typeof event.peerHasMediaPubKey === "boolean") {
            received.peerHasMediaPubKey = event.peerHasMediaPubKey;
          }
          if (typeof event.peerHasMediaSecret === "boolean") {
            received.peerHasMediaSecret = event.peerHasMediaSecret;
          }
          if (typeof event.peerMediaKeyIdIsZero === "boolean") {
            received.peerMediaKeyIdIsZero = event.peerMediaKeyIdIsZero;
          }
          if (
            typeof event.peerMediaKeyIdBits === "number" &&
            Number.isInteger(event.peerMediaKeyIdBits) &&
            event.peerMediaKeyIdBits >= 0 && event.peerMediaKeyIdBits <= 64
          ) {
            received.peerMediaKeyIdBits = event.peerMediaKeyIdBits;
          }
          if (Array.isArray(event.peerMedia)) {
            received.peerMedia = event.peerMedia.slice(0, 6).map((item) => {
              const rec = item && typeof item === "object"
                ? item as Record<string, unknown>
                : {};
              const num = (v: unknown) =>
                typeof v === "number" && Number.isSafeInteger(v) ? v : null;
              return {
                name: typeof rec.name === "string" &&
                    /^[A-Z]{1,4}$/.test(rec.name)
                  ? rec.name
                  : null,
                enabled: num(rec.enabled),
                kind: num(rec.kind),
                rtpId: num(rec.rtpId),
                rtpPort: num(rec.rtpPort),
                rtcpId: num(rec.rtcpId),
              };
            });
          }
          if (
            typeof event.dataSendMode === "string" &&
            dataSendModes.has(event.dataSendMode)
          ) {
            received.dataSendMode = event.dataSendMode;
          }
          if (
            typeof event.ssrc === "number" &&
            Number.isSafeInteger(event.ssrc) && event.ssrc >= 0
          ) {
            received.txSsrc = event.ssrc;
          }
          if (
            typeof event.payloadType === "number" &&
            Number.isInteger(event.payloadType) && event.payloadType >= 0 &&
            event.payloadType <= 127
          ) {
            received.txPayloadType = event.payloadType;
          }
          break;
        }
        case "mc_data_req_spec": {
          first = received.mcDataReqSpec === null;
          const rawSpec = event.spec && typeof event.spec === "object"
            ? event.spec as Record<string, unknown>
            : {};
          const num = (v: unknown) =>
            typeof v === "number" && Number.isSafeInteger(v) ? v : null;
          const bool = (v: unknown) => (typeof v === "boolean" ? v : null);
          const cleanStrms = Array.isArray(rawSpec.strms)
            ? rawSpec.strms.slice(0, 6).map((s) => {
              const r = s && typeof s === "object"
                ? s as Record<string, unknown>
                : {};
              return {
                ssrc: num(r.ssrc),
                paused: bool(r.paused),
                code: num(r.code),
                ptime: num(r.ptime),
              };
            })
            : [];
          const cleanTxStrms = Array.isArray(rawSpec.txStrms)
            ? rawSpec.txStrms.slice(0, 6).map((s) => {
              const r = s && typeof s === "object"
                ? s as Record<string, unknown>
                : {};
              return {
                ssrc: num(r.ssrc),
                paused: bool(r.paused),
                code: num(r.code),
              };
            })
            : [];
          received.mcDataReqSpec = {
            dispatchId: num(event.dispatchId),
            dataBytes: num(event.dataBytes),
            headerHex: typeof rawSpec.headerHex === "string" &&
                hexPattern.test(rawSpec.headerHex)
              ? rawSpec.headerHex
              : null,
            strms: cleanStrms,
            txStrms: cleanTxStrms,
            tp: num(rawSpec.tp),
            fbOn: bool(rawSpec.fbOn),
            fbIntv: num(rawSpec.fbIntv),
          };
          break;
        }
        case "data_rtp_send":
        case "group_data_rtp_send":
          received.dataRtpSent++;
          first = received.dataRtpSent === 1;
          break;
        case "media_endpoint_learned":
          received.endpointLearned++;
          first = received.endpointLearned === 1;
          break;
        case "media_endpoint_learn_skipped":
          received.endpointLearnSkipped++;
          first = received.endpointLearnSkipped === 1;
          break;
        case "media_send": {
          received.txMediaPackets++;
          first = received.txMediaPackets === 1;
          if (
            typeof event.bytes === "number" &&
            Number.isSafeInteger(event.bytes) && event.bytes >= 0
          ) {
            received.txMediaBytes += event.bytes;
          }
          if (
            received.txSsrc === null && typeof event.ssrc === "number" &&
            Number.isSafeInteger(event.ssrc) && event.ssrc >= 0
          ) {
            received.txSsrc = event.ssrc;
          }
          if (
            received.txPayloadType === null &&
            typeof event.payloadType === "number" &&
            Number.isInteger(event.payloadType) && event.payloadType >= 0 &&
            event.payloadType <= 127
          ) {
            received.txPayloadType = event.payloadType;
          }
          if (
            received.txRtpFirstByte === null &&
            typeof event.rtpFirstByte === "number" &&
            Number.isInteger(event.rtpFirstByte) && event.rtpFirstByte >= 0 &&
            event.rtpFirstByte <= 255
          ) {
            received.txRtpFirstByte = event.rtpFirstByte;
          }
          break;
        }
        case "media_decrypt_fail": {
          received.decryptFailures++;
          first = received.decryptFailures === 1;
          const reason = typeof event.reason === "string" &&
              failureReasons.has(event.reason)
            ? event.reason
            : "other";
          if (first) {
            received.firstFailure = reason;
          }
          if (received.decryptFailSamples.length < 3) {
            const wLen = typeof event.bytes === "number" &&
                Number.isSafeInteger(event.bytes)
              ? event.bytes
              : -1;
            const wHead = typeof event.wireHeadHex === "string" &&
                hexPattern.test(event.wireHeadHex)
              ? event.wireHeadHex
              : "";
            received.decryptFailSamples.push(`${reason}|${wLen}|${wHead}`);
          }
          break;
        }
        default:
          return;
      }
      if (first || Date.now() >= nextSummaryAt) {
        logSummary("receiving");
        nextSummaryAt = Date.now() + 5000;
      }
    },
  };
  logSummary("monitoring");
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
    pairedOpus: true,
    onEnded: (error) => {
      if (!current(call)) return;
      if (error) void failCall(call, error);
      else void endCall(call);
    },
  });
  await call.audioStart;
  if (!current(call)) return;
  if (
    call.kind === "VIDEO" &&
    typeof transport.sendVideo === "function" &&
    typeof transport.receiveVideo === "function"
  ) {
    call.videoSession = (runtime.startVideo ?? startCallVideoIO)({
      transport: {
        sendVideo: (payload, opts) => transport.sendVideo!(payload, opts),
        receiveVideo: () => transport.receiveVideo!(),
      },
      signal: call.abort.signal,
      controls: callVideoCtl,
      videoDevice: callVideoInput,
      onFrame: (side, jpeg) => {
        if (!current(call)) return;
        runtime.emitEvent("call_video_frame", {
          callId: call.id,
          side,
          ...(jpeg ? { jpeg: encodeJpegBase64(jpeg) } : { cleared: true }),
        });
      },
    });
  }
  state(call, "connected");
}

export async function doCallStart(
  id: number | string | null,
  peerMid: string,
  callType: CallKind = "AUDIO",
) {
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
  const kind: CallKind = callType === "VIDEO" ? "VIDEO" : "AUDIO";
  const call = reserve(`out-${crypto.randomUUID()}`, peerMid, kind);
  try {
    if (await runtime.loadAuthDevice() === "DESKTOPWIN") {
      if (current(call)) runtime.fail(id, "relogin_android_required");
      await endCall(call, true);
      return;
    }
    if (!current(call)) return;
    const localMid = client.base.profile?.mid;
    if (!localMid) throw new Error("profile not ready");
    runtime.ok(id, { callId: call.id, peer: peerMid, kind, state: "starting" });
    state(call, "acquiring");
    const route = await client.call.acquireRoute({
      to: peerMid,
      callType: kind,
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

async function connectIncoming(call: ActiveCall, offer: IncomingCall) {
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
function offerFresh(offer: IncomingCall) {
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
  const call = reserve(offer.callId, offer.callerMid, offer.kind);
  runtime.ok(id, {
    answering: true,
    peer: call.peer,
    callId: call.id,
    kind: call.kind,
  });
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
    const call = reserve(offer.callId, offer.callerMid, offer.kind);
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
      kind: offer.kind,
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
  const offer = parseIncomingCall(ev.raw, localMid);
  if (!offer || !offerFresh(offer) || seenIncoming.has(offer.callId)) return;
  seenIncoming.set(offer.callId, offer.createdAt + 120_000);
  if (seenIncoming.size > 256) {
    seenIncoming.delete(seenIncoming.keys().next().value ?? "");
  }
  incomingOffer = offer;
  runtime.emitEvent("call_incoming", {
    callId: offer.callId,
    from: offer.callerMid,
    kind: offer.kind,
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
