import { assert, assertEquals, assertRejects } from "@std/assert";
import { Buffer } from "node:buffer";
import {
  aesCtrEncrypt,
  buildDirectionLabel,
  buildPlanetCtrIv,
  deriveCallKeys,
  generateEphemeralKeypair,
  hmacTag,
} from "../vendor/linejs-call/planet/crypto.ts";
import { makeChunkHdr } from "../vendor/linejs-call/planet/framing.ts";
import {
  CC_MSG,
  type CcSetupRsp,
  decodePlanetMsg,
  packCcSetupRsp,
  packPlanetCcMsg,
  packPlanetMsg,
  wrapCcMsg,
} from "../vendor/linejs-call/planet/schema.ts";
import { PlanetTransport } from "../vendor/linejs-call/planet/transport.ts";

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
