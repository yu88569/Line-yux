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
    const listener = createListener({
      getClient: () => client,
      onTransportStalled: () => {},
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
      emitEvent: (event, payload) => {
        events.push({ event, payload });
      },
    });
    listener.start();
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
    listener.stop();
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

Deno.test("listener treats SEND_MESSAGE ops as mine even when raw from is empty", async () => {
  for (const opType of ["SEND_MESSAGE", 25] as const) {
    for (
      const { to, expectedPeer } of [
        { to: "u-peer", expectedPeer: "u-peer" },
        { to: "c-group", expectedPeer: "c-group" },
      ]
    ) {
      const client = new Client(
        new BaseClient({
          device: "ANDROIDSECONDARY",
          fetch: () => {
            throw new Error("Network forbidden in listener test");
          },
        }),
      );
      Object.defineProperty(client, "listen", { value: () => {} });
      const emitted = Promise.withResolvers<void>();
      const events: Array<
        { event: string; payload?: Record<string, unknown> }
      > = [];
      const upserts: Record<string, unknown>[] = [];
      const listener = createListener({
        getClient: () => client,
        getMyMid: () => "u-self",
        onTransportStalled: () => {},
        setListening: () => {},
        summarizeTalkMessage: () =>
          Promise.resolve({
            id: `msg-${opType}-${to}`,
            from: "",
            to,
            mine: false,
            createdTime: 789,
            needsMedia: false,
            reply: { id: "orig-1", chatMid: "" },
          }),
        invalidateMessages: () => {},
        storeBoxCursor: () => {},
        upsertChatFromMessage: (message) => {
          upserts.push(message);
          emitted.resolve();
          return Promise.resolve();
        },
        hydrateLiveMedia: () => Promise.resolve(),
        emitEvent: (event, payload) => {
          events.push({ event, payload });
        },
      });
      listener.start();
      client.emit(
        "event",
        {
          type: opType,
          message: { id: `msg-${opType}-${to}`, from: "", to },
        } as never,
      );
      client.emit(
        "message",
        new TalkMessage({
          client,
          raw: {
            id: `msg-${opType}-${to}`,
            from: "",
            to,
            toType: to.startsWith("c") ? "GROUP" : "USER",
            createdTime: 789,
            deliveredTime: 789,
            text: "hi",
            hasContent: false,
            contentType: "NONE",
            contentPreview: "",
            contentMetadata: {},
            sessionId: 0,
            chunks: [],
            relatedMessageId: "",
            messageRelationType: 0,
            readCount: 0,
            relatedMessageServiceCode: "TALK",
            appExtensionType: 0,
            reactions: [],
            updatedTime: 789,
          } as never,
        }),
      );
      await emitted.promise;
      listener.stop();
      const msgEvent = events.find((e) => e.event === "message");
      const msg = msgEvent?.payload?.message as Record<string, unknown>;
      assertEquals(msg.mine, true);
      assertEquals(msg.from, "u-self");
      assertEquals(msg.to, expectedPeer);
      assertEquals(
        (msg.reply as Record<string, unknown>).chatMid,
        expectedPeer,
      );
      assertEquals(upserts[0]?.mine, true);
      assertEquals(upserts[0]?.from, "u-self");
    }
  }
});

Deno.test("listener handles SEND_CHAT_CHECKED ops and emits chat_read", async () => {
  for (const opType of ["SEND_CHAT_CHECKED", 40] as const) {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden in listener test");
        },
      }),
    );
    Object.defineProperty(client, "listen", { value: () => {} });
    const events: unknown[] = [];
    const readCalls: Array<{ chatMid: string; messageId: string }> = [];
    const notified = Promise.withResolvers<void>();
    const listener = createListener({
      getClient: () => client,
      onChatRead: (chatMid, messageId) => {
        readCalls.push({ chatMid, messageId });
        notified.resolve();
      },
      onTransportStalled: () => {},
      setListening: () => {},
      summarizeTalkMessage: () => Promise.resolve({}),
      invalidateMessages: () => {},
      storeBoxCursor: () => {},
      upsertChatFromMessage: () => Promise.resolve(),
      hydrateLiveMedia: () => Promise.resolve(),
      emitEvent: (event, payload) => {
        events.push({ event, payload });
      },
    });
    listener.start();
    client.emit(
      "event",
      {
        type: opType,
        param1: "u-peer-1",
        param2: "999888777",
      } as never,
    );
    await notified.promise;
    listener.stop();
    assertEquals(readCalls, [{
      chatMid: "u-peer-1",
      messageId: "999888777",
    }]);
    assertEquals(events, [
      { event: "listening", payload: undefined },
      {
        event: "chat_read",
        payload: { chatMid: "u-peer-1", messageId: "999888777" },
      },
    ]);
  }
});

Deno.test("listener routes only explicit contact/profile ops and ignores stranger notifications", async () => {
  const client = new Client(
    new BaseClient({
      device: "ANDROIDSECONDARY",
      fetch: () => {
        throw new Error("Network forbidden in listener test");
      },
    }),
  );
  Object.defineProperty(client, "listen", { value: () => {} });
  const contactAdded: string[] = [];
  const profileUpdated: string[] = [];
  const listener = createListener({
    getClient: () => client,
    onTransportStalled: () => {},
    setListening: () => {},
    summarizeTalkMessage: () => Promise.resolve({}),
    invalidateMessages: () => {},
    storeBoxCursor: () => {},
    upsertChatFromMessage: () => Promise.resolve(),
    hydrateLiveMedia: () => Promise.resolve(),
    onContactAdded: (mid) => {
      contactAdded.push(mid);
    },
    onProfileUpdated: (mid) => {
      profileUpdated.push(mid);
    },
    emitEvent: () => {},
  });
  listener.start();
  try {
    for (
      const opType of [
        "NOTIFIED_UPDATE_PROFILE",
        2,
        "NOTIFIED_BUDDY_UPDATE_PROFILE",
        78,
      ] as const
    ) {
      client.emit(
        "event",
        { type: opType, param1: `u-prof-${opType}` } as never,
      );
    }
    for (
      const opType of [
        "ADD_CONTACT",
        4,
        "FRIEND_REQUEST_ACCEPTED",
        63,
      ] as const
    ) {
      client.emit(
        "event",
        { type: opType, param1: `u-add-${opType}` } as never,
      );
    }
    for (
      const opType of [
        "NOTIFIED_ADD_CONTACT",
        5,
        "NOTIFIED_FRIEND_REQUEST",
        88,
        "NOTIFIED_RECOMMEND_CONTACT",
        8,
      ] as const
    ) {
      client.emit(
        "event",
        { type: opType, param1: `u-stranger-${opType}` } as never,
      );
    }
    await Promise.resolve();
    assertEquals(profileUpdated, [
      "u-prof-NOTIFIED_UPDATE_PROFILE",
      "u-prof-2",
      "u-prof-NOTIFIED_BUDDY_UPDATE_PROFILE",
      "u-prof-78",
    ]);
    assertEquals(contactAdded, [
      "u-add-ADD_CONTACT",
      "u-add-4",
      "u-add-FRIEND_REQUEST_ACCEPTED",
      "u-add-63",
    ]);
  } finally {
    listener.stop();
  }
});
