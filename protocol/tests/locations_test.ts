import { assertEquals } from "@std/assert";
import {
  LOCATION_METADATA_VERSION,
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
      { ...location, latitude: "" },
      { ...location, latitude: "  " },
      { ...location, latitude: "0x10" },
      { ...location, latitude: "Infinity" },
      { ...location, latitude: false },
      { ...location, latitude: "NaN" },
      { ...location, longitude: "181" },
    ]
  ) assertEquals(parseLocation(value), null);
  assertEquals(parseLocation({ latitude: "0", longitude: "-120" }), {
    latitude: 0,
    longitude: -120,
    title: "",
    address: "",
  });
  assertEquals(
    needsLocationMetadataRefresh([{ contentType: "LOCATION" }]),
    true,
  );
  assertEquals(
    needsLocationMetadataRefresh([{
      contentType: "LOCATION",
      locationMetadataVersion: LOCATION_METADATA_VERSION,
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
      locationMetadataVersion: LOCATION_METADATA_VERSION,
    },
  }]);
  await service.sendLocation(2, mid, { ...location, latitude: 100 });
  await service.sendLocation(3, "", location);
  assertEquals(calls.length, 1);
  assertEquals(results.slice(1), ["invalid_location", "invalid_chat"]);
});

Deno.test("location summaries preserve plaintext and unavailable points", async () => {
  const { locationPayload } = await import("../src/locations.ts");
  assertEquals(locationPayload({ location }, "LOCATION"), {
    location,
    locationMetadataVersion: LOCATION_METADATA_VERSION,
  });
  assertEquals(locationPayload({}, "LOCATION"), {
    location: null,
    locationMetadataVersion: LOCATION_METADATA_VERSION,
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
      messages: [{
        id: "123",
        contentType: "LOCATION",
        text: "[LOCATION]",
        location: null,
        locationMetadataVersion: 1,
      }],
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
