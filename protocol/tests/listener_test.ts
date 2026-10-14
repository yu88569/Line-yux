import { assertEquals } from "@std/assert";
import { Client, TalkMessage } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { createListener } from "../src/listener.ts";

for (
  const { name, from, to, mine, expected } of [
    {
      name: "incoming group",
      from: "u-member",
      to: "c-group",
      mine: false,
      expected: "c-group",
    },
    {
      name: "outgoing group",
      from: "u-self",
      to: "c-group",
      mine: true,
      expected: "c-group",
    },
    {
      name: "incoming room",
      from: "u-member",
      to: "r-room",
      mine: false,
      expected: "r-room",
    },
    {
      name: "outgoing room",
      from: "u-self",
      to: "r-room",
      mine: true,
      expected: "r-room",
    },
    {
      name: "incoming direct",
      from: "u-member",
      to: "u-self",
      mine: false,
      expected: "u-member",
    },
    {
      name: "outgoing direct",
      from: "u-self",
      to: "u-member",
      mine: true,
      expected: "u-member",
    },
  ]
) {
  Deno.test(`listener routes ${name} message to its chat`, async () => {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden in listener test");
        },
      }),
    );
    Object.defineProperty(client, "listen", { value: () => {} });
    const summary = {
      id: "123",
      from,
      to,
      mine,
      createdTime: 456,
      needsMedia: true,
    };
    const invalidated: string[] = [];
    const cursors: unknown[] = [];
    const upserts: unknown[] = [];
    const mediaChats: string[] = [];
    const events: unknown[] = [];
    const hydrated = Promise.withResolvers<void>();
    createListener({
      getClient: () => client,
      isListening: () => false,
      setListening: () => {},
      summarizeTalkMessage: () => Promise.resolve(summary),
      invalidateMessages: (mid) => {
        invalidated.push(mid);
      },
      storeBoxCursor: (mid, cursor) => {
        cursors.push({ mid, ...cursor });
      },
      upsertChatFromMessage: (message) => {
        upserts.push(message);
        return Promise.resolve();
      },
      hydrateLiveMedia: (_message, _summary, mid) => {
        mediaChats.push(mid);
        hydrated.resolve();
        return Promise.resolve();
      },
      upsertChatFromContact: () => Promise.resolve(),
      emitEvent: (event, payload) => {
        events.push({ event, payload });
      },
    }).start();
    client.emit(
      "message",
      new TalkMessage({
        client,
        raw: {
          id: "123",
          from,
          to,
          toType: to.startsWith("c")
            ? "GROUP"
            : to.startsWith("r")
            ? "ROOM"
            : "USER",
          createdTime: 456,
          deliveredTime: 456,
          text: "",
          hasContent: true,
          contentType: "IMAGE",
          contentPreview: "",
          contentMetadata: {},
          sessionId: 0,
          chunks: [],
          relatedMessageId: "",
          messageRelationType: 0,
          readCount: 0,
          relatedMessageServiceCode: "TALK",
          appExtensionType: 1,
          reactions: [],
          updatedTime: 456,
          location: {
            title: "",
            address: "",
            latitude: 0,
            longitude: 0,
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
          },
        },
      }),
    );
    await hydrated.promise;
    assertEquals(invalidated, [expected]);
    assertEquals(cursors, [{
      mid: expected,
      messageId: "123",
      deliveredTime: 456,
    }]);
    assertEquals(mediaChats, [expected]);
    assertEquals(upserts, [summary]);
    assertEquals(events, [
      { event: "listening", payload: undefined },
      {
        event: "message",
        payload: { message: { ...summary, imagePath: null } },
      },
    ]);
  });
}
