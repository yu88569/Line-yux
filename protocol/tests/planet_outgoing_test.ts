import { assert, assertEquals, assertRejects } from "@std/assert";
import { Buffer } from "node:buffer";
import {
  aesCtrEncrypt,
  buildDirectionLabel,
  buildPlanetCtrIv,
  deriveCallKeys,
  derivePlanetMediaKeyingVariants,
  derivePlanetMediaStreamKeying,
  generateEphemeralKeypair,
  hmacTag,
} from "../vendor/linejs-call/planet/crypto.ts";
import { makeChunkHdr } from "../vendor/linejs-call/planet/framing.ts";
import {
  CC_MSG,
  type CcSetupRsp,
  decodeFields,
  decodeMcDataRsp,
  decodePlanetMsg,
  MC_MSG,
  packCcConnReq,
  packCcSetupRsp,
  packMcDataReq,
  packMcDataSessionPayload,
  packNativeAnswer,
  packPlanetCcMsg,
  packPlanetMcMsg,
  packPlanetMsg,
  packStrmSpec,
  wrapCcMsg,
  wrapMcMsg,
} from "../vendor/linejs-call/planet/schema.ts";
import { PlanetTransport } from "../vendor/linejs-call/planet/transport.ts";
import {
  deriveSrtpContext,
  parseRtp,
  srtpDecrypt,
} from "../vendor/linejs-call/srtp.ts";

async function outgoingFixture(response?: CcSetupRsp) {
  const server = generateEphemeralKeypair();
  const local = generateEphemeralKeypair();
  const self = `u${"1".repeat(32)}`, peer = `u${"2".repeat(32)}`;
  const seed = new Uint8Array(16).fill(7);
  const keys = deriveCallKeys({
    local: server,
    mpkey: local.publicKey,
    bootstrapSeed: seed,
    sendLabel: 7,
    recvLabel: 7,
  }).send;
  const sent: Uint8Array[] = [];
  const events: Record<string, unknown>[] = [];
  const transport = new PlanetTransport({
    localMid: self,
    transportKeypair: local,
    timeoutMs: 30,
    debug: (event) => events.push(event),
    wireSend: (_wire, endpoint) => {
      sent.push(endpoint.plaintext);
      if (!endpoint.bootstrap || response === undefined) return;
      const request = decodePlanetMsg(endpoint.plaintext);
      assert(request.hdr && request.cc?.hdr);
      assertEquals(request.cc.bodyTag, CC_MSG.SETUP_REQ);
      const plain = packPlanetMsg({
        userId: self,
        msgId: 0x2241,
        sessId: new Uint8Array(16).fill(8),
        tranId: request.hdr.tranId ?? new Uint8Array(16),
        tranSeq: request.hdr.tranSeq ?? 1,
        locNonce: 555n,
        rmtNonce: request.hdr.locNonce ?? 0n,
      }, {
        kind: "cc",
        data: packPlanetCcMsg({
          cid: request.cc.hdr.cid ?? "synthetic-call",
          srcChanId: 444n,
          dstChanId: request.cc.hdr.srcChanId ?? 0n,
        }, wrapCcMsg(CC_MSG.SETUP_RSP, packCcSetupRsp(response))),
      });
      const sequence = 1;
      const prefix = Buffer.concat([
        buildDirectionLabel(7),
        seed,
        server.publicKey,
      ]);
      const security = new Uint8Array([
        0,
        0,
        0,
        0x28 | ((plain.length >>> 8) & 7),
        plain.length & 255,
      ]);
      const cipher = aesCtrEncrypt(
        keys.encKey,
        buildPlanetCtrIv(keys.ctrBase, sequence),
        plain,
      );
      const total = 6 + prefix.length + security.length + cipher.length + 16;
      const chunk = makeChunkHdr((((total - 4) << 5) | 0x1d) & 65535);
      const header = new Uint8Array([
        chunk & 255,
        (chunk >>> 8) & 255,
        0,
        sequence,
        6,
        2,
      ]);
      const authenticated = Buffer.concat([header, prefix, security, cipher]);
      return Buffer.concat([
        authenticated,
        hmacTag(keys.macKey, authenticated),
      ]);
    },
  });
  await transport.connect({
    route: {
      voipAddress: "127.0.0.1",
      voipAddress6: "",
      voipUdpPort: 9000,
      commParam: JSON.stringify({
        mpkey: Buffer.from(server.publicKey).toString("base64"),
      }),
      fromToken: "synthetic-token",
      fromZone: "JP",
      toZone: "JP",
      toMid: peer,
    },
  });
  return { transport, peer, sent, events };
}

for (
  const response of [
    { result: 1, relCode: 403, relPhrase: "private server phrase" },
    { result: 1 },
    { result: 0, relCode: 403 },
    { relCode: 403 },
  ]
) {
  Deno.test(`outgoing rejects SETUP result ${response.result} relCode ${response.relCode} before media probes`, async () => {
    const fixture = await outgoingFixture(response);
    try {
      const error = await assertRejects(
        () => fixture.transport.inviteDetailed({ to: fixture.peer }),
        Error,
        "PLANET SETUP rejected",
      );
      assert(!error.message.includes("private server phrase"));
      assertEquals(fixture.sent.length, 1);
      const event = fixture.events.find((event) => event.type === "setup_rsp");
      assertEquals(event?.result, response.result);
      assertEquals(event?.relCode, response.relCode);
      assert(!JSON.stringify(event).includes("private server phrase"));
    } finally {
      await fixture.transport.close();
    }
  });
}

for (const response of [{ result: 0, relCode: 0 }, {}]) {
  Deno.test(`outgoing accepts SETUP without rejection ${JSON.stringify(response)}`, async () => {
    const fixture = await outgoingFixture(response);
    try {
      const result = await fixture.transport.inviteDetailed({
        to: fixture.peer,
      });
      assertEquals(result.setupRsp?.result, response.result);
      assertEquals(fixture.sent.length, 19);
    } finally {
      await fixture.transport.close();
    }
  });
}

Deno.test("outgoing SETUP timeout does not send media probes", async () => {
  const fixture = await outgoingFixture();
  try {
    await assertRejects(
      () => fixture.transport.inviteDetailed({ to: fixture.peer }),
      Error,
      "PLANET reply timeout",
    );
    assertEquals(fixture.sent.length, 1);
    assertEquals(
      fixture.events.filter((event) => event.type === "setup_rsp").length,
      0,
    );
  } finally {
    await fixture.transport.close();
  }
});

Deno.test("outgoing 1:1 uses peer answer field 61 SSRCs (202/222), M=1 on packet 0, and txStrms [202, 212]", async () => {
  const server = generateEphemeralKeypair();
  const local = generateEphemeralKeypair();
  const peerMediaKey = generateEphemeralKeypair();
  const peerMediaNonce = new Uint8Array(16).fill(0x55);
  const peerMediaSecret = new Uint8Array(30).fill(0x66);
  const peerAnswer = packNativeAnswer({
    mediaPubKey: peerMediaKey.publicKey,
    mediaKeyId: 123456,
    mediaNonce: peerMediaNonce,
    mediaSecret: peerMediaSecret,
  });
  const self = `u${"1".repeat(32)}`, peer = `u${"2".repeat(32)}`;
  const seed = new Uint8Array(16).fill(7);
  const sessId = new Uint8Array(16).fill(8);
  const keys = deriveCallKeys({
    local: server,
    mpkey: local.publicKey,
    bootstrapSeed: seed,
    sendLabel: 7,
    recvLabel: 7,
  }).send;

  let seq = 1;
  let callId = "";
  let localCcChanId = 0n;
  let localNonce = 0n;
  let connReqQueued = false;
  let mcDataReqSent = false;
  const mediaWires: Uint8Array[] = [];
  let mcDataRspBody: Uint8Array | undefined;

  function packServerRegular(plain: Uint8Array): Uint8Array {
    const s = seq++;
    const cipher = aesCtrEncrypt(
      keys.encKey,
      buildPlanetCtrIv(keys.ctrBase, s),
      plain,
    );
    const total = 6 + cipher.length + 16;
    const chunk = makeChunkHdr((((total - 4) << 5) | 0x0d) & 65535);
    const tail = (((0x18 | ((plain.length >>> 8) & 7)) << 8) |
      (plain.length & 255)) & 65535;
    const header = new Uint8Array([
      chunk & 255,
      (chunk >>> 8) & 255,
      (s >>> 8) & 255,
      s & 255,
      (tail >>> 8) & 255,
      tail & 255,
    ]);
    const authenticated = Buffer.concat([header, cipher]);
    return Buffer.concat([authenticated, hmacTag(keys.macKey, authenticated)]);
  }

  const transport = new PlanetTransport({
    localMid: self,
    transportKeypair: local,
    mediaKeyMode: "audio-reverse-stage",
    timeoutMs: 1000,
    wireSend: (wire, endpoint) => {
      if (endpoint.bootstrap) {
        const request = decodePlanetMsg(endpoint.plaintext);
        assert(request.hdr && request.cc?.hdr);
        callId = request.cc.hdr.cid ?? "synthetic-call";
        localCcChanId = request.cc.hdr.srcChanId ?? 0n;
        localNonce = request.hdr.locNonce ?? 0n;
        const plain = packPlanetMsg({
          userId: self,
          msgId: 0x2241,
          sessId,
          tranId: request.hdr.tranId ?? new Uint8Array(16),
          tranSeq: request.hdr.tranSeq ?? 1,
          locNonce: 555n,
          rmtNonce: localNonce,
        }, {
          kind: "cc",
          data: packPlanetCcMsg({
            cid: callId,
            srcChanId: 444n,
            dstChanId: localCcChanId,
          }, wrapCcMsg(CC_MSG.SETUP_RSP, packCcSetupRsp({ result: 0 }))),
        });
        const s = seq++;
        const prefix = Buffer.concat([
          buildDirectionLabel(7),
          seed,
          server.publicKey,
        ]);
        const security = new Uint8Array([
          0,
          0,
          0,
          0x28 | ((plain.length >>> 8) & 7),
          plain.length & 255,
        ]);
        const cipher = aesCtrEncrypt(
          keys.encKey,
          buildPlanetCtrIv(keys.ctrBase, s),
          plain,
        );
        const total = 6 + prefix.length + security.length + cipher.length + 16;
        const chunk = makeChunkHdr((((total - 4) << 5) | 0x1d) & 65535);
        const header = new Uint8Array([
          chunk & 255,
          (chunk >>> 8) & 255,
          0,
          s,
          6,
          2,
        ]);
        const authenticated = Buffer.concat([header, prefix, security, cipher]);
        return Buffer.concat([
          authenticated,
          hmacTag(keys.macKey, authenticated),
        ]);
      }
      if ((wire[0] & 0xc0) === 0x80) {
        mediaWires.push(new Uint8Array(wire));
        return;
      }
      const msg = decodePlanetMsg(endpoint.plaintext);
      if (msg.scBytes && !connReqQueued) {
        // Trigger responder CONN_REQ right after keepalive.
        connReqQueued = true;
        return packServerRegular(
          packPlanetMsg({
            userId: self,
            msgId: 0x2144,
            sessId,
            tranId: new Uint8Array(16).fill(2),
            tranSeq: 2,
            locNonce: 555n,
            rmtNonce: localNonce,
          }, {
            kind: "cc",
            data: packPlanetCcMsg(
              {
                cid: callId,
                srcChanId: 444n,
                dstChanId: localCcChanId,
              },
              wrapCcMsg(
                CC_MSG.CONN_REQ,
                packCcConnReq({
                  answer: peerAnswer,
                  mChanId: 666n,
                  netType: 1,
                  unavailToSec: 120,
                  oCapas: [],
                  features: [],
                }),
              ),
            ),
          }),
        );
      }
      if (msg.cc?.bodyTag === CC_MSG.CONN_RSP && !mcDataReqSent) {
        mcDataReqSent = true;
        return packServerRegular(
          packPlanetMsg({
            userId: self,
            msgId: 0x3189,
            sessId,
            tranId: new Uint8Array(16).fill(3),
            tranSeq: 3,
            locNonce: 555n,
            rmtNonce: localNonce,
          }, {
            kind: "mc",
            data: packPlanetMcMsg(
              {
                cid: callId,
                srcChanId: 666n,
                dstChanId: localCcChanId,
              },
              wrapMcMsg(
                MC_MSG.DATA_REQ,
                packMcDataReq({
                  srcType: 0,
                  dstType: 0,
                  dispatchId: 2,
                  data: packMcDataSessionPayload(
                    packStrmSpec({
                      strms: [{ ssrc: 202, ptime: 40 }],
                      txStrms: [{ ssrc: 102 }],
                    }),
                  ),
                }),
              ),
            ),
          }),
        );
      }
      if (msg.mc?.bodyTag === MC_MSG.DATA_RSP && msg.mc.bodyBytes) {
        mcDataRspBody = msg.mc.bodyBytes;
      }
    },
  });

  try {
    await transport.connect({
      route: {
        voipAddress: "127.0.0.1",
        voipAddress6: "",
        voipUdpPort: 9000,
        commParam: JSON.stringify({
          mpkey: Buffer.from(server.publicKey).toString("base64"),
        }),
        fromToken: "synthetic-token",
        fromZone: "JP",
        toZone: "JP",
        toMid: peer,
      },
    });
    await transport.inviteDetailed({ to: peer });
    const answer = await transport.waitForAnswerDetailed({
      timeoutMs: 1000,
      autoConnRsp: true,
    });
    assertEquals(answer.mediaReady, true);

    await transport.send(new Uint8Array([0x00, 0x78, 0x01]), {
      timestampStep: 1920,
    });
    await transport.send(new Uint8Array([0x00, 0x78, 0x02]), {
      timestampStep: 1920,
    });

    const localOffer = transport.localMediaOffer!;
    const peerKeys = derivePlanetMediaKeyingVariants({
      local: {
        privateKey: peerMediaKey.privateKey,
        publicKey: peerMediaKey.publicKey,
        mediaKeyId: 123456,
        mediaNonce: peerMediaNonce,
      },
      peer: {
        publicKey: localOffer.material.mediaPubKey,
        mediaKeyId: localOffer.material.mediaKeyId,
        mediaNonce: localOffer.material.mediaNonce,
      },
    });
    const peerAudioRecv = await deriveSrtpContext(
      derivePlanetMediaStreamKeying(
        peerKeys.variants["local-peer/peer"],
        "AUDIO",
      ),
    );
    const peerDataRecv = await deriveSrtpContext(
      derivePlanetMediaStreamKeying(
        peerKeys.variants["local-peer/peer"],
        "DATA",
      ),
    );

    const audioPackets = mediaWires.filter((w) => (w[1] & 0x7f) === 96);
    const dataPackets = mediaWires.filter((w) => (w[1] & 0x7f) === 98);
    assertEquals(audioPackets.length, 2);
    assert(dataPackets.length >= 1);

    const a0 = parseRtp(await srtpDecrypt(peerAudioRecv, audioPackets[0]));
    const a1 = parseRtp(await srtpDecrypt(peerAudioRecv, audioPackets[1]));
    assertEquals(a0.ssrc, 202);
    assertEquals(a0.marker, true);
    assertEquals(a1.ssrc, 202);
    assertEquals(a1.marker, false);

    const d0 = parseRtp(await srtpDecrypt(peerDataRecv, dataPackets[0]));
    assertEquals(d0.ssrc, 222);

    assert(mcDataRspBody);
    const rsp = decodeMcDataRsp(mcDataRspBody);
    assert(rsp.data && rsp.data.length > 8);
    const specLen = (rsp.data[4] << 8) | rsp.data[5];
    const specFields = decodeFields(rsp.data.subarray(8, 8 + specLen));
    const rxSsrcs = specFields
      .filter((f) => f.tag === 1 && f.value instanceof Uint8Array)
      .map((f) =>
        Number(
          decodeFields(f.value as Uint8Array).find((x) => x.tag === 1)?.value,
        )
      );
    const txSsrcs = specFields
      .filter((f) => f.tag === 6 && f.value instanceof Uint8Array)
      .map((f) =>
        Number(
          decodeFields(f.value as Uint8Array).find((x) => x.tag === 1)?.value,
        )
      );
    assertEquals(rxSsrcs, [102, 112, 122]);
    assertEquals(txSsrcs, [202, 212]);
  } finally {
    await transport.close();
  }
});
