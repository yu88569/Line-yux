import { assert, assertEquals, assertRejects } from "@std/assert";
import { Buffer } from "node:buffer";
import { createSocket, type RemoteInfo, type Socket } from "node:dgram";
import {
  aesCtrDecrypt,
  aesCtrEncrypt,
  buildDirectionLabel,
  buildPlanetCtrIv,
  deriveCallKeys,
  derivePlanetMediaKeyingVariants,
  derivePlanetMediaStreamKeying,
  generateEphemeralKeypair,
  hmacTag,
  tagEquals,
  type TransportKeys,
} from "../vendor/linejs-call/planet/crypto.ts";
import { makeChunkHdr } from "../vendor/linejs-call/planet/framing.ts";
import {
  CC_MSG,
  decodeCcConnReq,
  decodeFields,
  decodeNativeSetupOffer,
  decodePlanetMsg,
  encodeVarint,
  MC_MSG,
  packCcConnRsp,
  packCcRelReq,
  packNativeSetupOffer,
  packPlanetCcMsg,
  packPlanetMcMsg,
  packPlanetMsg,
  type PlanetSetupOfferMaterial,
  wrapCcMsg,
  wrapMcMsg,
} from "../vendor/linejs-call/planet/schema.ts";
import { PlanetTransport } from "../vendor/linejs-call/planet/transport.ts";
import { parseIncomingAudioCall } from "../vendor/linejs-call/incoming.ts";
import {
  buildRtp,
  deriveSrtpContext,
  parseRtp,
  srtpDecrypt,
  srtpEncrypt,
} from "../vendor/linejs-call/srtp.ts";
const HEADER_LEN = 6;
const BOOTSTRAP_PREFIX_LEN = 51;
const BOOTSTRAP_SEC_HEADER_LEN = 5;
const REGULAR_TAIL_CONTROL_BASE = 0x18;

function bytesToBase64(bytes: Uint8Array): string {
  let raw = "";
  for (const b of bytes) raw += String.fromCharCode(b);
  return btoa(raw);
}

function concatBytes(parts: Uint8Array[]): Uint8Array {
  const len = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(len);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

function buildBootstrapSecHeader(plaintextLen: number): Uint8Array {
  return new Uint8Array([
    0,
    0,
    0,
    0x28 | ((plaintextLen >>> 8) & 0x07),
    plaintextLen & 0xff,
  ]);
}

function buildBootstrapFrameHeader(
  totalLen: number,
  sequence: number,
): Uint8Array {
  const chunkLogical = ((((totalLen - 4) << 5) | 0x1d) & 0xffff) >>> 0;
  const chunk = makeChunkHdr(chunkLogical);
  return new Uint8Array([
    chunk & 0xff,
    (chunk >>> 8) & 0xff,
    (sequence >>> 8) & 0xff,
    sequence & 0xff,
    0x06,
    0x02,
  ]);
}

function buildRegularFrameHeader(
  totalLen: number,
  sequence: number,
  plaintextLen: number,
): Uint8Array {
  const chunkLogical = ((((totalLen - 4) << 5) | 0x0d) & 0xffff) >>> 0;
  const chunk = makeChunkHdr(chunkLogical);
  const tail16 = (((REGULAR_TAIL_CONTROL_BASE |
    ((plaintextLen >>> 8) & 0x07)) << 8) | (plaintextLen & 0xff)) & 0xffff;
  return new Uint8Array([
    chunk & 0xff,
    (chunk >>> 8) & 0xff,
    (sequence >>> 8) & 0xff,
    sequence & 0xff,
    (tail16 >>> 8) & 0xff,
    tail16 & 0xff,
  ]);
}

function buildServerWire(
  keys: TransportKeys,
  plaintext: Uint8Array,
  sequence: number,
  opts: { bootstrap?: { label: number; seed: Uint8Array; pub: Uint8Array } } =
    {},
): Uint8Array {
  const ct = aesCtrEncrypt(
    keys.encKey,
    buildPlanetCtrIv(keys.ctrBase, sequence),
    plaintext,
  );
  const tagLen = 16;
  if (opts.bootstrap) {
    const prefix = concatBytes([
      buildDirectionLabel(opts.bootstrap.label),
      opts.bootstrap.seed,
      opts.bootstrap.pub,
    ]);
    assertEquals(prefix.length, BOOTSTRAP_PREFIX_LEN);
    const sec = buildBootstrapSecHeader(plaintext.length);
    assertEquals(sec.length, BOOTSTRAP_SEC_HEADER_LEN);
    const totalLen = HEADER_LEN + prefix.length + sec.length + ct.length +
      tagLen;
    const hdr = buildBootstrapFrameHeader(totalLen, sequence);
    const macInput = concatBytes([hdr, prefix, sec, ct]);
    return concatBytes([macInput, hmacTag(keys.macKey, macInput)]);
  }
  const totalLen = HEADER_LEN + ct.length + tagLen;
  const hdr = buildRegularFrameHeader(totalLen, sequence, plaintext.length);
  const macInput = concatBytes([hdr, ct]);
  return concatBytes([macInput, hmacTag(keys.macKey, macInput)]);
}

function extractBootstrapClientPub(wire: Uint8Array): Uint8Array {
  return wire.subarray(
    HEADER_LEN + 2 + 16,
    HEADER_LEN + 2 + 16 + 33,
  );
}

function extractBootstrapClientLabel(wire: Uint8Array): number {
  return ((wire[HEADER_LEN] << 8) | wire[HEADER_LEN + 1]) & 0xffff;
}

function extractBootstrapClientSeed(wire: Uint8Array): Uint8Array {
  return new Uint8Array(wire.subarray(HEADER_LEN + 2, HEADER_LEN + 18));
}

function decryptRegularWire(
  keys: TransportKeys,
  wire: Uint8Array,
): Uint8Array | undefined {
  const tag = wire.subarray(wire.length - 16);
  const macInput = wire.subarray(0, wire.length - 16);
  const expected = hmacTag(keys.macKey, macInput);
  if (!tagEquals(tag, expected)) return undefined;
  const seq = ((wire[2] << 8) | wire[3]) & 0xffff;
  const ct = wire.subarray(HEADER_LEN, wire.length - 16);
  return aesCtrDecrypt(keys.encKey, buildPlanetCtrIv(keys.ctrBase, seq), ct);
}

function isRtpLike(wire: Uint8Array): boolean {
  return wire.length >= 12 && (wire[0] & 0xc0) === 0x80;
}

async function bindUdpServer(): Promise<Socket> {
  const server = createSocket("udp4");
  await new Promise<void>((resolve) =>
    server.bind({ address: "127.0.0.1", port: 0 }, () => resolve())
  );
  return server;
}

async function sendUdp(
  server: Socket,
  packet: Uint8Array,
  rinfo: RemoteInfo,
): Promise<void> {
  await new Promise<void>((resolve, reject) =>
    server.send(
      Buffer.from(packet),
      rinfo.port,
      rinfo.address,
      (err) => err ? reject(err) : resolve(),
    )
  );
}

function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error(`${label} timeout`)),
      timeoutMs,
    );
    promise.then(
      (value) => {
        clearTimeout(timeout);
        resolve(value);
      },
      (err) => {
        clearTimeout(timeout);
        reject(err);
      },
    );
  });
}

async function exerciseIncoming(
  mode: "media" | "timeout" | "abort" | "bad-address" | "bad-offer",
) {
  const relay = await bindUdpServer();
  const attacker = await bindUdpServer();
  const routeKey = generateEphemeralKeypair();
  const peerKey = generateEphemeralKeypair();
  const self = `u${"2".repeat(32)}`, caller = `u${"1".repeat(32)}`;
  const address = relay.address();
  assert(typeof address !== "string");
  const peerMaterial: PlanetSetupOfferMaterial = {
    mediaPubKey: peerKey.publicKey,
    mediaKeyId: 345,
    mediaNonce: new Uint8Array(16).fill(3),
    mediaSecret: new Uint8Array(30).fill(4),
  };
  const operation = {
    type: "NOTIFIED_RECEIVED_CALL",
    param1: caller,
    param2: "synthetic-token",
    createdTime: 1,
    param3: JSON.stringify({
      k: "CA",
      n: "synthetic-token",
      vs: "synthetic-call",
      vfz: "JP",
      vtz: "JP",
      h: "127.0.0.1",
      p: address.port,
      vc: JSON.stringify({ mpkey: bytesToBase64(routeKey.publicKey) }),
    }),
  };
  const call = parseIncomingAudioCall(operation, self)!;
  const abort = new AbortController();
  const identity = {
    deviceId: bytesToBase64(new Uint8Array(32).fill(7)),
    signal: abort.signal,
    timeoutMs: 300,
  };
  const sessId = new Uint8Array(16).fill(8), seed = new Uint8Array(16).fill(9);
  let recvKeys: TransportKeys, remote: RemoteInfo;
  let sendKeys!: TransportKeys;
  let sequence = 1;
  const outgoing: ReturnType<typeof decodePlanetMsg>[] = [];
  const answerSeen = Promise.withResolvers<
    ReturnType<typeof decodePlanetMsg>
  >();
  const mediaSeen = Promise.withResolvers<Uint8Array>();
  const joinSeen = Promise.withResolvers<ReturnType<typeof decodePlanetMsg>>();
  const releaseSeen = Promise.withResolvers<
    ReturnType<typeof decodePlanetMsg>
  >();
  let serverError: unknown;
  let transport: PlanetTransport | undefined;
  const field = (bytes: Uint8Array, tag: number) =>
    decodeFields(bytes).find((f) => f.tag === tag)?.value;
  const reply = async (
    request: ReturnType<typeof decodePlanetMsg>,
    bodyTag: number,
    bodyBytes: Uint8Array,
    msgId: number,
    options: { foreign?: boolean; bootstrap?: boolean; socket?: Socket } = {},
  ) => {
    const h = request.hdr!;
    const plain = packPlanetMsg({
      userId: self,
      msgId,
      sessId,
      locNonce: 555n,
      rmtNonce: h.locNonce!,
      tranId: options.foreign ? new Uint8Array(16).fill(99) : h.tranId!,
      tranSeq: h.tranSeq!,
    }, {
      kind: "cc",
      data: packPlanetCcMsg({
        cid: call.callId,
        srcChanId: 444n,
        dstChanId: request.cc!.hdr!.srcChanId!,
      }, wrapCcMsg(bodyTag, bodyBytes)),
    });
    await sendUdp(
      options.socket ?? relay,
      buildServerWire(
        sendKeys,
        plain,
        sequence++,
        options.bootstrap
          ? { bootstrap: { label: 7, seed, pub: routeKey.publicKey } }
          : {},
      ),
      remote,
    );
  };
  relay.on("message", (buf, rinfo) => {
    void (async () => {
      const wire = new Uint8Array(buf);
      if (isRtpLike(wire)) {
        mediaSeen.resolve(wire);
        return;
      }
      remote = rinfo;
      let plain: Uint8Array | undefined;
      if (!recvKeys) {
        const pub = extractBootstrapClientPub(wire),
          label = extractBootstrapClientLabel(wire);
        recvKeys = deriveCallKeys({
          local: routeKey,
          mpkey: pub,
          bootstrapSeed: extractBootstrapClientSeed(wire),
          sendLabel: label,
          recvLabel: label,
        }).recv;
        sendKeys = deriveCallKeys({
          local: routeKey,
          mpkey: pub,
          bootstrapSeed: seed,
          sendLabel: 7,
          recvLabel: 7,
        }).send;
        assert(
          tagEquals(
            wire.subarray(-16),
            hmacTag(recvKeys.macKey, wire.subarray(0, -16)),
          ),
        );
        plain = aesCtrDecrypt(
          recvKeys.encKey,
          buildPlanetCtrIv(recvKeys.ctrBase, (wire[2] << 8) | wire[3]),
          wire.subarray(62, -16),
        );
      } else plain = decryptRegularWire(recvKeys, wire);
      assert(plain);
      if (plain.length === 10 || plain.length === 519) return;
      const msg = decodePlanetMsg(plain);
      if (msg.mc?.bodyTag === MC_MSG.JOIN_RSP) joinSeen.resolve(msg);
      outgoing.push(msg);
      if (msg.cc?.bodyTag === CC_MSG.VERIFY_REQ) {
        assertEquals(msg.hdr?.msgId, 0x2142);
        assertEquals(msg.hdr?.sessId?.length ?? 0, 0);
        assertEquals(
          new TextDecoder().decode(field(msg.cc.bodyBytes!, 1) as Uint8Array),
          caller,
        );
        assertEquals(
          new TextDecoder().decode(field(msg.cc.bodyBytes!, 2) as Uint8Array),
          self,
        );
        assertEquals(field(msg.cc.bodyBytes!, 9), call.credential);
        assertEquals(
          new TextDecoder().decode(field(msg.cc.bodyBytes!, 10) as Uint8Array),
          "freecall.audio",
        );
        const offer = mode === "bad-offer"
          ? new Uint8Array([0])
          : packNativeSetupOffer(peerMaterial);
        const body = concatBytes([
          new Uint8Array([8, 0, 50]),
          encodeVarint(offer.length),
          offer,
        ]);
        // Authenticated but unrelated traffic must not bind the endpoint/session.
        await reply(msg, CC_MSG.VERIFY_RSP, body, 0x2242, {
          bootstrap: true,
          foreign: true,
          socket: attacker,
        });
        await reply(msg, CC_MSG.VERIFY_RSP, body, 0x2242, { bootstrap: true });
      } else if (msg.cc?.bodyTag === CC_MSG.CONN_REQ) {
        assertEquals(msg.hdr?.msgId, 0x2144);
        assertEquals(msg.hdr?.sessId, sessId);
        assertEquals(msg.cc.hdr?.dstChanId, 444n);
        await reply(
          msg,
          CC_MSG.CONN_RSP,
          packCcConnRsp({ mChanId: 666n }),
          0x2344,
        );
        await reply(
          msg,
          CC_MSG.CONN_RSP,
          packCcConnRsp({ result: 0, mChanId: 666n }),
          0x2244,
          { foreign: true },
        );
        answerSeen.resolve(msg);
      } else if (
        msg.cc?.bodyTag === CC_MSG.REL_REQ || msg.cc?.bodyTag === CC_MSG.REL_RSP
      ) releaseSeen.resolve(msg);
    })().catch((e) => {
      serverError = e;
      answerSeen.reject(e);
    });
  });
  try {
    transport = new PlanetTransport({
      localMid: self,
      timeoutMs: identity.timeoutMs,
    });
    await transport.connect({ route: call.route });
    const active = transport;
    const accepting = (async () => {
      await active.verifyIncomingDetailed(call, {
        deviceId: identity.deviceId,
        userAgent: { osName: "Windows", osVersion: "1", deviceName: "Desktop" },
      }, abort.signal);
      await active.acceptIncomingDetailed();
      return active;
    })();
    const failure = mode !== "media"
      ? assertRejects(
        () => accepting,
        Error,
        mode === "timeout"
          ? "timeout"
          : mode === "bad-address"
          ? "endpoint unresolved"
          : mode === "bad-offer"
          ? "unsupported incoming media offer"
          : undefined,
      )
      : undefined;
    if (mode === "bad-offer") {
      await failure;
      assertEquals(
        outgoing.some((m) => m.cc?.bodyTag === CC_MSG.CONN_REQ),
        false,
      );
      assertEquals(serverError, undefined);
      return;
    }
    const conn = await withTimeout(answerSeen.promise, 1000, "CONN request");
    await assertRejects(
      () =>
        active.verifyIncomingDetailed(call, {
          deviceId: identity.deviceId,
          userAgent: {
            osName: "Windows",
            osVersion: "1",
            deviceName: "Desktop",
          },
        }),
      Error,
      "incoming state already started",
    );
    if (mode === "abort") {
      abort.abort();
      await failure;
    } else if (mode === "timeout") await failure;
    else if (mode === "bad-address") {
      await reply(
        conn,
        CC_MSG.CONN_RSP,
        packCcConnRsp({
          result: 0,
          mChanId: 666n,
          mAddr: { ip: "127.0.0.1", port: 0 },
        }),
        0x2244,
      );
      await failure;
    } else {
      let accepted = false;
      void accepting.then(() => {
        accepted = true;
      });
      await new Promise((r) => setTimeout(r, 15));
      assertEquals(
        accepted,
        false,
        "provisional/foreign CONN must not enable media",
      );
      await reply(
        conn,
        CC_MSG.CONN_RSP,
        packCcConnRsp({
          result: 0,
          mChanId: 666n,
          uePublicAddr: { ip: "127.0.0.1", port: 9 },
        }),
        0x2244,
      );
      transport = await accepting;
      const answer = decodeNativeSetupOffer(
        decodeCcConnReq(conn.cc!.bodyBytes!).answer!,
      );
      assertEquals(answer.mediaSecret, undefined);
      assertEquals(answer.media.find((m) => m.name === "V")?.enabled, 0);
      assert(!tagEquals(answer.mediaPubKey!, peerMaterial.mediaPubKey));
      const keys = derivePlanetMediaKeyingVariants({
        local: {
          privateKey: peerKey.privateKey,
          publicKey: peerKey.publicKey,
          mediaKeyId: peerMaterial.mediaKeyId,
          mediaNonce: peerMaterial.mediaNonce,
        },
        peer: {
          publicKey: answer.mediaPubKey!,
          mediaKeyId: answer.mediaKeyId!,
          mediaNonce: answer.mediaNonce!,
        },
      });
      const peerRecv = await deriveSrtpContext(
        derivePlanetMediaStreamKeying(
          keys.variants["local-peer/peer"],
          "AUDIO",
        ),
      );
      const peerSend = await deriveSrtpContext(
        derivePlanetMediaStreamKeying(
          keys.variants["peer-local/local"],
          "AUDIO",
        ),
      );
      const frames = transport.receive()[Symbol.asyncIterator]();
      const next = frames.next();
      await sendUdp(
        attacker,
        buildRtp({
          payloadType: 96,
          ssrc: 777,
          seq: 1,
          timestamp: 1,
          payload: new Uint8Array([9]),
        }),
        remote!,
      );
      await sendUdp(
        relay,
        await srtpEncrypt(
          peerSend,
          buildRtp({
            payloadType: 96,
            ssrc: 101,
            seq: 1,
            timestamp: 960,
            payload: new Uint8Array([1, 2]),
          }),
        ),
        remote!,
      );
      assertEquals(
        (await withTimeout(next, 1000, "callee receive")).value,
        new Uint8Array([1, 2]),
      );
      await transport.send(new Uint8Array([3, 4]));
      const audio = parseRtp(
        await srtpDecrypt(
          peerRecv,
          await withTimeout(mediaSeen.promise, 1000, "callee send"),
        ),
      );
      assertEquals(audio.payload, new Uint8Array([3, 4]));
      assertEquals(
        audio.ssrc,
        answer.media.find((m) => m.name === "A")!.rtpPort,
        "send uses local answer field 11",
      );
      assertEquals(audio.ssrc, 102);
      await sendUdp(
        relay,
        buildServerWire(
          sendKeys,
          packPlanetMsg({
            userId: self,
            msgId: 0x3185,
            sessId,
            locNonce: 555n,
            rmtNonce: conn.hdr!.locNonce!,
            tranId: new Uint8Array(16).fill(12),
            tranSeq: 12,
          }, {
            kind: "mc",
            data: packPlanetMcMsg({
              cid: call.callId,
              srcChanId: 666n,
              dstChanId: decodeCcConnReq(conn.cc!.bodyBytes!).mChanId!,
            }, wrapMcMsg(MC_MSG.JOIN_REQ, new Uint8Array([8, 0]))),
          }),
          sequence++,
        ),
        remote!,
      );
      const join = await withTimeout(
        joinSeen.promise,
        1000,
        "media JOIN response",
      );
      const spec = field(join.mc!.bodyBytes!, 17);
      assert(spec instanceof Uint8Array);
      const streams = decodeFields(spec).filter((f) => f.tag === 6).map((f) => {
        assert(f.value instanceof Uint8Array);
        return field(f.value, 1);
      });
      assertEquals(streams, [202n, 212n]);
      const pending = frames.next();
      await reply(
        conn,
        CC_MSG.REL_REQ,
        packCcRelReq({ relCode: 1, releaser: "initiator" }),
        0x2145,
      );
      assertEquals((await transport.ended).by, "remote");
      assertEquals((await pending).done, true);
      await assertRejects(
        () => transport!.send(new Uint8Array([1])),
        Error,
        "released by remote",
      );
    }
    const release = await withTimeout(releaseSeen.promise, 1000, "release");
    assertEquals(
      release.cc?.bodyTag,
      mode === "media" ? CC_MSG.REL_RSP : CC_MSG.REL_REQ,
    );
    if (mode === "media") {
      assertEquals(release.hdr?.tranId, conn.hdr?.tranId);
      assertEquals(release.hdr?.tranSeq, conn.hdr?.tranSeq);
    } else {assertEquals(
        new TextDecoder().decode(
          field(release.cc!.bodyBytes!, 3) as Uint8Array,
        ),
        "responder",
      );}
    assertEquals(
      outgoing.some((m) => m.cc?.bodyTag === CC_MSG.SETUP_REQ),
      false,
    );
    assertEquals(serverError, undefined);
  } finally {
    abort.abort();
    await transport?.close();
    await new Promise<void>((r) => relay.close(() => r()));
    await new Promise<void>((r) => attacker.close(() => r()));
  }
}

Deno.test("incoming answer authenticates UDP session, sends local SSRC and ends on release", () =>
  exerciseIncoming("media"));
Deno.test("incoming provisional-only timeout releases responder and closes socket", () =>
  exerciseIncoming("timeout"));
Deno.test("incoming abort while answering releases responder and closes socket", () =>
  exerciseIncoming("abort"));
Deno.test("incoming explicit malformed media address never falls back", () =>
  exerciseIncoming("bad-address"));
Deno.test("incoming malformed verified offer cannot send an answer", () =>
  exerciseIncoming("bad-offer"));

Deno.test("closing while UDP connect yields never reopens a socket", async () => {
  const self = `u${"2".repeat(32)}`;
  const call = parseIncomingAudioCall({
    type: 50,
    param1: `u${"1".repeat(32)}`,
    param2: "test-token",
    createdTime: Date.now(),
    param3: JSON.stringify({
      k: "CA",
      n: "test-token",
      vs: "bind-cancel",
      vfz: "JP",
      vtz: "JP",
      h: "127.0.0.1",
      p: 9000,
      vc: JSON.stringify({
        mpkey: bytesToBase64(generateEphemeralKeypair().publicKey),
      }),
    }),
  }, self);
  assert(call);
  const transport = new PlanetTransport({ localMid: self });
  const connecting = transport.connect({ route: call.route });
  const rejected = assertRejects(() => connecting, Error);
  await transport.close();
  await rejected;
  await transport.close();
});
