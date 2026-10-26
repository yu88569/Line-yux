import { assertEquals } from "@std/assert";
import { BaseClient } from "@evex/linejs/base";
import { Client, TalkMessage } from "@evex/linejs";
import nacl from "npm:tweetnacl@^1.0.3";
import { Buffer } from "node:buffer";
import { patchMessageDecryption } from "../src/message_decryption.ts";
import { locationPayload } from "../src/locations.ts";

const sender = "u11111111111111111111111111111111";
const receiver = "u22222222222222222222222222222222";
const group = "c33333333333333333333333333333333";
const point = {
  title: "",
  address: "Test place",
  latitude: 0,
  longitude: -120.5,
};

const nativePoint: Parameters<
  BaseClient["e2ee"]["encryptE2EELocationMessage"]
>[4] = {
  ...point,
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
};

function encryptedFixture(options: {
  mine: boolean;
  group: boolean;
  contentType: "LOCATION" | 15;
  stringCoordinates: boolean;
}) {
  const senderKey = nacl.box.keyPair.fromSecretKey(new Uint8Array(32).fill(11));
  const receiverKey = nacl.box.keyPair.fromSecretKey(
    new Uint8Array(32).fill(22),
  );
  const groupKey = nacl.box.keyPair.fromSecretKey(new Uint8Array(32).fill(33));
  const newerKey = nacl.box.keyPair.fromSecretKey(new Uint8Array(32).fill(99));
  const base = new BaseClient({
    device: "ANDROIDSECONDARY",
    fetch: () => {
      throw Error("network forbidden");
    },
  });
  const self = options.mine ? senderKey : receiverKey;
  const selfData = {
    privKey: Buffer.from(self.secretKey).toString("base64"),
    pubKey: Buffer.from(self.publicKey).toString("base64"),
  };
  Object.defineProperty(base, "profile", {
    value: { mid: options.mine ? sender : receiver },
  });
  Object.defineProperty(base.e2ee, "getE2EESelfKeyData", {
    value: () =>
      Promise.resolve(
        options.group ? selfData : {
          privKey: Buffer.from(newerKey.secretKey).toString("base64"),
          pubKey: Buffer.from(newerKey.publicKey).toString("base64"),
        },
      ),
  });
  Object.defineProperty(base.e2ee, "getE2EESelfKeyDataByKeyId", {
    value: (keyId: number) => {
      assertEquals(keyId, options.mine ? 11 : 22);
      return Promise.resolve(selfData);
    },
  });
  Object.defineProperty(base.e2ee, "getE2EELocalPublicKey", {
    value: (mid: string, keyId: number) => {
      if (mid === group) {
        assertEquals(keyId, 33);
        return Promise.resolve({
          privKey: Buffer.from(groupKey.secretKey).toString("base64"),
          keyId: 33,
        });
      }
      if (mid === sender && keyId === 11) {
        return Promise.resolve(Buffer.from(senderKey.publicKey));
      }
      if (mid === receiver && keyId === 22) {
        return Promise.resolve(Buffer.from(receiverKey.publicKey));
      }
      throw Error("wrong peer key");
    },
  });
  const to = options.group ? group : receiver;
  const shared = options.group
    ? base.e2ee.generateSharedSecret(
      Buffer.from(groupKey.secretKey),
      Buffer.from(senderKey.publicKey),
    )
    : base.e2ee.generateSharedSecret(
      Buffer.from(senderKey.secretKey),
      Buffer.from(receiverKey.publicKey),
    );
  const chunks = options.stringCoordinates
    ? base.e2ee.encryptE2EEMessageByData(
      11,
      options.group ? 33 : 22,
      Buffer.from(shared),
      2,
      {
        location: {
          latitude: "0",
          longitude: "-120.5",
          address: point.address,
        },
      },
      to,
      sender,
      15,
    )
    : base.e2ee.encryptE2EELocationMessage(
      11,
      options.group ? 33 : 22,
      Buffer.from(shared),
      2,
      nativePoint,
      to,
      sender,
    );
  const raw = {
    id: "123",
    from: sender,
    to,
    toType: options.group ? "GROUP" : "USER",
    contentType: options.contentType,
    contentMetadata: { e2eeVersion: "2" },
    chunks,
  } as unknown as Parameters<typeof TalkMessage.fromRawTalk>[0];
  patchMessageDecryption(base.e2ee);
  return { base, client: new Client(base), raw };
}

Deno.test("location decrypt preserves plaintext and missing-key fallback", async () => {
  const base = new BaseClient({
    device: "ANDROIDSECONDARY",
    fetch: () => {
      throw Error("missing key fixture cannot use network");
    },
  });
  const client = new Client(base);
  patchMessageDecryption(base.e2ee);
  const { raw } = encryptedFixture({
    mine: false,
    group: false,
    contentType: "LOCATION",
    stringCoordinates: true,
  });
  const plain: Parameters<typeof TalkMessage.fromRawTalk>[0] = {
    ...raw,
    chunks: [],
    location: nativePoint,
  };
  const plainMessage = await TalkMessage.fromRawTalk(plain, client);
  assertEquals(locationPayload(plainMessage.raw, "LOCATION").location, point);
  const unavailable = await TalkMessage.fromRawTalk(raw, client);
  assertEquals(locationPayload(unavailable.raw, "LOCATION").location, null);
  const text = {
    ...raw,
    contentType: "NONE" as const,
    chunks: [],
    text: "Text",
  };
  const textMessage = await TalkMessage.fromRawTalk({
    ...text,
    contentMetadata: {},
  }, client);
  assertEquals(textMessage.text, "Text");
});

for (const mine of [false, true]) {
  for (const isGroup of [false, true]) {
    for (const contentType of ["LOCATION", 15] as const) {
      Deno.test(`real encrypted location ${mine ? "sent" : "received"} ${isGroup ? "group" : "DM"} ${contentType}`, async () => {
        const { client, raw } = encryptedFixture({
          mine,
          group: isGroup,
          contentType,
          stringCoordinates: !mine,
        });
        const message = await TalkMessage.fromRawTalk(raw, client);
        assertEquals(locationPayload(message.raw, "LOCATION").location, point);
      });
    }
  }
}
