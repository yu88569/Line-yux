import { assert, assertEquals } from "@std/assert";
import {
  AnnexBNaluSplitter,
  H264RtpDepacketizer,
  MjpegFrameSplitter,
  packetizeH264Nalu,
} from "../src/call_video.ts";
import { parseIncomingCall } from "../vendor/linejs-call/incoming.ts";
import { generateEphemeralKeypair } from "../vendor/linejs-call/planet/crypto.ts";

Deno.test("AnnexBNaluSplitter extracts 3-byte and 4-byte start-code NALUs across chunks", () => {
  const splitter = new AnnexBNaluSplitter();
  const sps = new Uint8Array([0x67, 0x42, 0xe0, 0x1f]);
  const pps = new Uint8Array([0x68, 0xce, 0x3c, 0x80]);
  const idr = new Uint8Array([0x65, 0x88, 0x84, 0x21, 0x43]);
  const stream = new Uint8Array([
    0x00,
    0x00,
    0x00,
    0x01,
    ...sps,
    0x00,
    0x00,
    0x01,
    ...pps,
    0x00,
    0x00,
    0x00,
    0x01,
    ...idr,
  ]);
  const out: Uint8Array[] = [];
  // Feed in tiny 3-byte chunks to stress boundary handling
  for (let i = 0; i < stream.length; i += 3) {
    out.push(...splitter.push(stream.subarray(i, i + 3)));
  }
  out.push(...splitter.flush());
  assertEquals(out, [sps, pps, idr]);
});

Deno.test("H.264 RTP packetize + depacketize roundtrips single NALU and FU-A fragments", () => {
  const depack = new H264RtpDepacketizer();
  // Single NALU (SPS)
  const sps = new Uint8Array([0x67, 0x42, 0xc0, 0x1e, 0xd9]);
  const singlePkts = packetizeH264Nalu(sps, 100);
  assertEquals(singlePkts.length, 1);
  assertEquals(singlePkts[0].isLastFragment, true);
  assertEquals(depack.push(singlePkts[0].payload), [
    new Uint8Array([0, 0, 0, 1, ...sps]),
  ]);

  // Large IDR NALU fragmented via FU-A
  const largeIdr = new Uint8Array(260);
  largeIdr[0] = 0x65; // NRI=3, type=5 (IDR)
  for (let i = 1; i < largeIdr.length; i++) largeIdr[i] = i & 0xff;
  const frags = packetizeH264Nalu(largeIdr, 80);
  assert(frags.length > 3);
  assertEquals(frags.at(-1)!.isLastFragment, true);
  assertEquals(frags[0].isLastFragment, false);

  const reassembled: Uint8Array[] = [];
  for (const frag of frags) {
    reassembled.push(...depack.push(frag.payload));
  }
  assertEquals(reassembled.length, 1);
  assertEquals(reassembled[0], new Uint8Array([0, 0, 0, 1, ...largeIdr]));
});

Deno.test("H264RtpDepacketizer handles STAP-A and strips 1-byte PLANET prefix", () => {
  const depack = new H264RtpDepacketizer();
  const sps = new Uint8Array([0x67, 0x42, 0xe0]);
  const pps = new Uint8Array([0x68, 0xce]);
  const stapA = new Uint8Array([
    0x78, // NRI=3, type=24 (STAP-A)
    0x00,
    sps.length,
    ...sps,
    0x00,
    pps.length,
    ...pps,
  ]);
  assertEquals(depack.push(stapA), [
    new Uint8Array([0, 0, 0, 1, ...sps]),
    new Uint8Array([0, 0, 0, 1, ...pps]),
  ]);

  // Leading non-H.264 prefix byte (0x00 -> nalType 0) before valid single NALU
  const prefixed = new Uint8Array([0x00, 0x65, 0xaa, 0xbb]);
  assertEquals(depack.push(prefixed), [
    new Uint8Array([0, 0, 0, 1, 0x65, 0xaa, 0xbb]),
  ]);
});

Deno.test("MjpegFrameSplitter extracts complete JPEG frames across chunks", () => {
  const splitter = new MjpegFrameSplitter();
  const f1 = new Uint8Array([0xff, 0xd8, 0x11, 0x22, 0xff, 0xd9]);
  const f2 = new Uint8Array([0xff, 0xd8, 0x33, 0x44, 0x55, 0xff, 0xd9]);
  const stream = new Uint8Array([...f1, ...f2]);
  const out: Uint8Array[] = [];
  for (let i = 0; i < stream.length; i += 4) {
    out.push(...splitter.push(stream.subarray(i, i + 4)));
  }
  assertEquals(out, [f1, f2]);
});

Deno.test("parseIncomingCall accepts both CA audio and CV video calls", () => {
  const caller = `u${"1".repeat(32)}`;
  const self = `u${"2".repeat(32)}`;
  const baseParams = {
    n: "tok",
    vs: "cid-video-1",
    vfz: "JP",
    vtz: "JP",
    h: "127.0.0.1",
    p: "9000",
    vc: JSON.stringify({
      mpkey: btoa(String.fromCharCode(...generateEphemeralKeypair().publicKey)),
    }),
  };
  const videoOp = {
    type: "NOTIFIED_RECEIVED_CALL",
    param1: caller,
    param2: "tok",
    param3: JSON.stringify({ ...baseParams, k: "CV" }),
    createdTime: 1000,
  };
  const parsedVideo = parseIncomingCall(videoOp, self);
  assert(parsedVideo);
  assertEquals(parsedVideo.kind, "VIDEO");
  assertEquals(parsedVideo.callId, "cid-video-1");

  const audioOp = {
    ...videoOp,
    param3: JSON.stringify({ ...baseParams, k: "CA" }),
  };
  const parsedAudio = parseIncomingCall(audioOp, self);
  assert(parsedAudio);
  assertEquals(parsedAudio.kind, "AUDIO");
});
