import { assertEquals } from "@std/assert";
import {
  needsLocationMetadataRefresh,
  parseLocation,
} from "../src/locations.ts";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";

const location = {
  title: "Place",
  address: "Address",
  latitude: 0,
  longitude: -120,
};
Deno.test("location validates coordinates without inventing missing points", () => {
  assertEquals(parseLocation(location), location);
  for (
    const value of [
      null,
      {},
      { ...location, latitude: 91 },
      { ...location, longitude: 181 },
      { ...location, latitude: NaN },
      { ...location, longitude: Infinity },
      { ...location, latitude: "0" },
    ]
  ) assertEquals(parseLocation(value), null);
  assertEquals(
    needsLocationMetadataRefresh([{ contentType: "LOCATION" }]),
    true,
  );
  assertEquals(
    needsLocationMetadataRefresh([{
      contentType: "LOCATION",
      locationMetadataVersion: 1,
      location: null,
    }]),
    false,
  );
});
Deno.test("native location send omits text so E2EE encrypts coordinates", async () => {
  const calls: unknown[] = [];
  const results: unknown[] = [];
  const runtime = {
    getClient: () => ({
      base: {
        talk: {
          sendMessage: (v: unknown) => {
            calls.push(v);
            return Promise.resolve({ id: "123" });
          },
        },
      },
    }),
    msgCache: new Map(),
    setChatCache: () => {},
    sentMessagePayload: () => ({ id: "123" }),
    touchChatPreviewFromMessage: () => {},
    ok: (_: unknown, r: unknown) => results.push(r),
    fail: (_: unknown, e: unknown) => results.push(e),
  } as unknown as CommandRuntime;
  const service = createCommandService(runtime);
  const mid = "u11111111111111111111111111111111";
  await service.sendLocation(1, mid, location);
  assertEquals(calls, [{ to: mid, contentType: "LOCATION", location }]);
  assertEquals(results, [{
    message: {
      id: "123",
      contentType: "LOCATION",
      location,
      locationMetadataVersion: 1,
    },
  }]);
  await service.sendLocation(2, mid, { ...location, latitude: 100 });
  await service.sendLocation(3, "", location);
  assertEquals(calls.length, 1);
  assertEquals(results.slice(1), ["invalid_location", "invalid_chat"]);
});

Deno.test("native E2EE location reaches normalized receive payload", async () => {
  const { BaseClient } = await import("@evex/linejs/base");
  const { Client, TalkMessage } = await import("@evex/linejs");
  const { locationPayload } = await import("../src/locations.ts");
  const base = new BaseClient({
    device: "ANDROIDSECONDARY",
    fetch: () => {
      throw Error("network forbidden");
    },
  });
  const client = new Client(base);
  let decrypts = 0;
  Object.defineProperty(base.e2ee, "decryptE2EELocationMessage", {
    value: () => {
      decrypts++;
      return Promise.resolve(location);
    },
  });
  const raw = {
    id: "123",
    contentType: "LOCATION",
    contentMetadata: { e2eeVersion: "2" },
    chunks: [],
    from: "u111",
    to: "u222",
  } as unknown as Parameters<typeof TalkMessage.fromRawTalk>[0];
  const message = await TalkMessage.fromRawTalk(raw, client);
  assertEquals(decrypts, 1);
  assertEquals(locationPayload(message.raw, "LOCATION"), {
    location,
    locationMetadataVersion: 1,
  });
  assertEquals(locationPayload({ location }, "LOCATION"), {
    location,
    locationMetadataVersion: 1,
  });
  assertEquals(locationPayload({}, "LOCATION"), {
    location: null,
    locationMetadataVersion: 1,
  });
  assertEquals(locationPayload({ location }, "NONE"), {
    location: null,
    locationMetadataVersion: null,
  });
});

Deno.test("legacy location cache refetches instead of returning stripped points forever", async () => {
  const mid = "u11111111111111111111111111111111";
  let fetched = false;
  const runtime = {
    getClient: () => ({
      base: {
        talk: {
          getPreviousMessagesV2WithRequest: () => {
            fetched = true;
            return Promise.resolve([]);
          },
        },
      },
    }),
    msgCache: new Map([[mid, {
      at: Date.now(),
      messages: [{ id: "123", contentType: "LOCATION", text: "[LOCATION]" }],
    }]]),
    boxCursor: new Map([[mid, { messageId: 1, deliveredTime: 1 }]]),
    refreshCachePolicy: () => Promise.resolve({}),
    cachePolicy: () => ({ memMsg: 60000 }),
    dedupe: (_key: string, work: () => Promise<void>) => work(),
    bumpMediaEpoch: () => 0,
    loadDiskMessages: () => Promise.resolve(null),
    saveDiskMessages: () => Promise.resolve(),
    forUiMessages: (messages: unknown) => Promise.resolve(messages),
    hydrateMedia: () => Promise.resolve(),
    emitEvent: () => {},
    ok: () => {},
    fail: () => {},
  } as unknown as CommandRuntime;
  await createCommandService(runtime).fetchMessages(1, mid);
  assertEquals(fetched, true);
});
