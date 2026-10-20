import { assertEquals, assertRejects } from "@std/assert";
import {
  buildRtp,
  deriveSrtpContext,
  parseRtp,
  srtpDecrypt,
  srtpEncrypt,
} from "../vendor/linejs-call/srtp.ts";

const masterKeying = Uint8Array.from({ length: 30 }, (_, i) => i + 1);

async function encryptedPackets(sequences: number[], ssrc = 101) {
  const sender = await deriveSrtpContext(masterKeying);
  const packets = [];
  for (const seq of sequences) {
    const plain = buildRtp({
      payloadType: 111,
      seq,
      timestamp: seq * 960,
      ssrc,
      payload: Uint8Array.of(1, 2, 3, seq & 0xff),
    });
    packets.push({ plain, wire: await srtpEncrypt(sender, plain) });
  }
  return packets;
}

Deno.test("SRTP receives audio across sequence rollover", async () => {
  const receiver = await deriveSrtpContext(masterKeying);
  const packets = await encryptedPackets([65534, 65535, 0, 1]);
  for (const { plain, wire } of packets) {
    assertEquals(await srtpDecrypt(receiver, wire), plain);
  }
  assertEquals(receiver.rocs.get(101), 1);
});

Deno.test("SRTP detects rollover when the last old-cycle packet is lost", async () => {
  const receiver = await deriveSrtpContext(masterKeying);
  const packets = await encryptedPackets([65534, 65535, 0, 1]);
  for (const i of [0, 2, 3]) {
    assertEquals(
      await srtpDecrypt(receiver, packets[i].wire),
      packets[i].plain,
    );
  }
});

Deno.test("SRTP accepts reordered packets on both sides of rollover without rewinding", async () => {
  const receiver = await deriveSrtpContext(masterKeying);
  const packets = await encryptedPackets([65533, 65534, 65535, 0, 1, 2]);
  for (const i of [0, 3, 2, 4, 1, 5]) {
    assertEquals(
      await srtpDecrypt(receiver, packets[i].wire),
      packets[i].plain,
    );
  }
  assertEquals(receiver.rocs.get(101), 1);
});

Deno.test("SRTP keeps rollover independent for each SSRC", async () => {
  const receiver = await deriveSrtpContext(masterKeying);
  const first = await encryptedPackets([65534, 65535, 0]);
  const second = await encryptedPackets([7, 8, 9], 202);
  for (let i = 0; i < first.length; i++) {
    assertEquals(await srtpDecrypt(receiver, first[i].wire), first[i].plain);
    assertEquals(await srtpDecrypt(receiver, second[i].wire), second[i].plain);
  }
  assertEquals(receiver.rocs.get(101), 1);
  assertEquals(receiver.rocs.get(202), 0);
});

Deno.test("SRTP invalid authentication cannot initialize or advance receiver state", async () => {
  const receiver = await deriveSrtpContext(masterKeying);
  const packets = await encryptedPackets([65534, 65535, 0, 1]);
  for (const i of [2, 0]) {
    const forged = packets[i].wire.slice();
    forged[forged.length - 1] ^= 1;
    await assertRejects(
      () => srtpDecrypt(receiver, forged),
      Error,
      "SRTP auth tag mismatch",
    );
  }
  assertEquals(receiver.rocs.size, 0);
  assertEquals(await srtpDecrypt(receiver, packets[0].wire), packets[0].plain);
  const forged = packets[2].wire.slice();
  forged[forged.length - 1] ^= 1;
  await assertRejects(
    () => srtpDecrypt(receiver, forged),
    Error,
    "SRTP auth tag mismatch",
  );
  assertEquals(receiver.rocs.get(101) ?? 0, 0);
  for (const i of [1, 2, 3]) {
    assertEquals(
      await srtpDecrypt(receiver, packets[i].wire),
      packets[i].plain,
    );
  }
});

Deno.test("SRTP duplicate old-cycle packets preserve the current receive index", async () => {
  const receiver = await deriveSrtpContext(masterKeying);
  const packets = await encryptedPackets([65534, 65535, 0, 1]);
  for (const i of [0, 1, 2, 1, 1, 3]) {
    assertEquals(
      await srtpDecrypt(receiver, packets[i].wire),
      packets[i].plain,
    );
  }
  assertEquals(receiver.rocs.get(101), 1);
});

Deno.test("SRTP honors an initial out-of-band ROC with reordered first packets", async () => {
  const sender = await deriveSrtpContext(masterKeying);
  const receiver = await deriveSrtpContext(masterKeying);
  sender.rocs.set(101, 7);
  receiver.rocs.set(101, 7);
  const packets = [];
  for (const seq of [65534, 65535, 0, 1]) {
    const plain = buildRtp({
      payloadType: 111,
      seq,
      timestamp: 0,
      ssrc: 101,
      payload: Uint8Array.of(seq & 0xff),
    });
    packets.push({ plain, wire: await srtpEncrypt(sender, plain) });
  }
  for (const i of [1, 0, 2, 3]) {
    assertEquals(
      await srtpDecrypt(receiver, packets[i].wire),
      packets[i].plain,
    );
  }
  assertEquals(receiver.rocs.get(101), 8);
});

Deno.test("parseRtp strips RFC 3550 trailing padding when P bit is set", () => {
  const unpadded = buildRtp({
    payloadType: 96,
    seq: 42,
    timestamp: 1920,
    ssrc: 102,
    payload: Uint8Array.of(0x10, 0x7b, 0x82, 0x01, 0xaa, 0xbb),
  });
  const padded = new Uint8Array(unpadded.length + 4);
  padded.set(unpadded, 0);
  padded[0] |= 0x20; // P bit
  padded[padded.length - 1] = 4; // 4 padding octets
  const parsed = parseRtp(padded);
  assertEquals(
    parsed.payload,
    Uint8Array.of(0x10, 0x7b, 0x82, 0x01, 0xaa, 0xbb),
  );
});
