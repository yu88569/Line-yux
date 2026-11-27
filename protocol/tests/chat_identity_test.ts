import { assertEquals } from "@std/assert";
import { Client } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { join } from "@std/path";
import { policyFor } from "../src/cache_policy.ts";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import {
  CHAT_CACHE_VERSION,
  type ChatCache,
  chatInfo,
  type ContactInfo,
  type ContactSyncRuntime,
  handleContactAdded,
  handleProfileUpdate,
  type IdentityRuntime,
  pruneUserContacts,
  resolveContactInfo,
  resolveSenderPicture,
} from "../src/chat_identity.ts";
import { resolveSenderAvatar } from "../src/sender_avatar.ts";

function fixture() {
  const calls: string[] = [];
  const runtime: IdentityRuntime = {
    contactIndex: new Map<string, ContactInfo>(),
    getTalk: () => ({
      getContact: ({ mid }) => {
        calls.push(`contact:${mid}`);
        if (!mid.startsWith("u")) {
          return Promise.reject(new Error("not contact"));
        }
        return Promise.resolve({ displayName: "User", pictureStatus: "/user" });
      },
      getChat: ({ chatMid, withMembers, withInvitees }) => {
        calls.push(`chat:${chatMid}:${withMembers}:${withInvitees}`);
        return Promise.resolve({
          chatName: "Group",
          picturePath: "/group",
          notificationDisabled: true,
        });
      },
    }),
    saveContacts: () => {
      calls.push("save");
      return Promise.resolve();
    },
  };
  return { runtime, calls };
}

Deno.test("live groups and rooms resolve their own name and photo", async () => {
  for (const mid of ["c-group", "r-room"]) {
    const { runtime, calls } = fixture();
    assertEquals(await resolveContactInfo(mid, runtime), {
      name: "Group",
      picturePath: "/group",
      kind: "group",
      muted: true,
    });
    assertEquals(calls, [`chat:${mid}:false:false`, "save"]);
    assertEquals(runtime.contactIndex.get(mid)?.name, "Group");
  }
});

Deno.test("joined group metadata supplies photo and prevents live refetch", async () => {
  const { runtime, calls } = fixture();
  const info = chatInfo("c-group", {
    chatName: "Joined group",
    picturePath: "/joined",
    notificationDisabled: true,
  });
  runtime.contactIndex.set("c-group", info);
  assertEquals(await resolveContactInfo("c-group", runtime), {
    name: "Joined group",
    picturePath: "/joined",
    kind: "group",
    muted: true,
  });
  assertEquals(calls, []);
});

Deno.test("user pictureStatus is usable without picturePath", async () => {
  const { runtime } = fixture();
  assertEquals(
    (await resolveContactInfo("u-user", runtime)).picturePath,
    "/user",
  );
});

Deno.test("named users with no photo remain cache hits", async () => {
  const { runtime, calls } = fixture();
  const cached = {
    name: "No photo",
    picturePath: null,
    kind: "dm",
    muted: false,
  };
  runtime.contactIndex.set("u-user", cached);
  assertEquals(await resolveContactInfo("u-user", runtime), cached);
  assertEquals(calls, []);
});

Deno.test("failed group lookup preserves fallback identity", async () => {
  const { runtime } = fixture();
  runtime.getTalk = () => ({
    getContact: () => Promise.reject(new Error("offline")),
    getChat: () => Promise.reject(new Error("offline")),
  });
  assertEquals(await resolveContactInfo("c-group", runtime), {
    name: "c-group",
    picturePath: null,
    kind: "group",
    muted: false,
  });
});

Deno.test("chat listing retains group activity and passes joined photos to hydration", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden in chat identity test");
        },
      }),
    );
    Object.defineProperty(client.base.talk, "getMessageBoxes", {
      value: () =>
        Promise.resolve({
          messageBoxes: ["c-group", "c-keep"].map((id) => ({
            id,
            lastDeliveredMessageId: { deliveredTime: 123 },
            unreadCount: 3,
          })),
        }),
    });
    Object.defineProperty(client, "fetchJoinedChats", {
      value: () =>
        Promise.resolve([
          {
            mid: "c-group",
            raw: {
              chatName: "Group",
              picturePath: "/new",
              notificationDisabled: true,
            },
          },
          { mid: "c-keep", raw: { chatName: "Keep", picturePath: "" } },
        ]),
    });
    const contactIndex = new Map<string, ContactInfo>([["c-keep", {
      name: "Keep",
      picturePath: "/old",
      kind: "group",
      muted: false,
    }]]);
    let cached: ReturnType<CommandRuntime["getChatCache"]> = {
      at: 0,
      chats: [{
        mid: "c-group",
        name: "c-group",
        kind: "group",
        avatarPath: null,
        lastActivity: 1,
        unread: 0,
        preview: "Existing preview",
      }],
    };
    let hydrated: Parameters<CommandRuntime["hydrateAvatars"]>[0] = [];
    let result: unknown;
    let saved = 0;
    const unused = () => {
      throw new Error("Unexpected command dependency");
    };
    const runtime: CommandRuntime = {
      getClient: () => client,
      setClient: unused,
      isListening: () => false,
      setAuthDead: unused,
      getChatCache: () => cached,
      setChatCache: (value) => {
        cached = value;
      },
      msgCache: new Map(),
      contactIndex,
      boxCursor: new Map(),
      avatarDir: directory,
      chatCachePath: join(directory, "chats.json"),
      storagePath: directory,
      lineDevice: "ANDROIDSECONDARY",
      lineVersion: "test",
      refreshCachePolicy: () => Promise.resolve(policyFor("smart")),
      cachePolicy: () => policyFor("smart"),
      dedupe: (_key, fn) => fn(),
      storeBoxCursor: () => {},
      existingFile: () => Promise.resolve(null),
      loadDiskContacts: () => Promise.resolve(),
      saveDiskContacts: () => {
        saved++;
        return Promise.resolve();
      },
      refreshContactIndex: () => Promise.resolve(),
      hydrateAvatars: (chats) => {
        hydrated = chats;
        return Promise.resolve();
      },
      hydratePreviews: () => Promise.resolve(),
      bumpMediaEpoch: unused,
      loadDiskMessages: unused,
      saveDiskMessages: unused,
      forUiMessages: unused,
      hydrateMedia: unused,
      summarizeTalkMessage: unused,
      summarizeRawMessage: unused,
      saveAuth: unused,
      loadAuth: unused,
      loadAuthDevice: unused,
      clearAuth: unused,
      myProfilePayload: unused,
      patchE2eeGuards: unused,
      startListen: unused,
      sentMessagePayload: unused,
      touchChatPreviewFromMessage: unused,
      emitEvent: () => {},
      ok: (_id, payload) => {
        result = payload;
      },
      fail: (_id, error) => {
        throw new Error(error);
      },
    };
    await createCommandService(runtime).listChats(1, true);
    assertEquals(result, { chats: hydrated, count: 2, cached: false });
    const row = hydrated.find((chat) => chat.mid === "c-group");
    assertEquals(row, {
      mid: "c-group",
      name: "Group",
      kind: "group",
      avatarPath: null,
      picturePath: "/new",
      lastActivity: 123,
      unread: 3,
      preview: "Existing preview",
      muted: true,
    });
    assertEquals(
      hydrated.find((chat) => chat.mid === "c-keep")?.picturePath,
      "/old",
    );
    assertEquals(contactIndex.get("c-group"), {
      name: "Group",
      picturePath: "/new",
      kind: "group",
      muted: true,
    });
    assertEquals(saved, 1);
    assertEquals(
      await Deno.readTextFile(join(directory, "chats.json")).then(JSON.parse),
      cached,
    );
    assertEquals(cached?.version, CHAT_CACHE_VERSION);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("resolveSenderPicture resolves non-friend avatar without polluting contactIndex", async () => {
  const sender = `u${"a".repeat(32)}`;
  const noPhotoSender = `u${"b".repeat(32)}`;
  const friend = `u${"c".repeat(32)}`;
  const contactIndex = new Map<string, ContactInfo>([
    [friend, {
      name: "Friend",
      picturePath: "/friend-pic",
      kind: "dm",
      muted: false,
    }],
  ]);
  const senderPictureCache = new Map<string, string | null>();
  const rpcCalls: string[] = [];
  const emitted: string[] = [];

  const getTalk = () => ({
    getContact: ({ mid }: { mid: string }) => {
      rpcCalls.push(mid);
      if (mid === sender) {
        return Promise.resolve({ pictureStatus: "/stranger" });
      }
      if (mid === noPhotoSender) return Promise.resolve({});
      return Promise.reject(new Error("unexpected"));
    },
  });

  const path = await resolveSenderAvatar(sender, {
    existingPath: () => Promise.resolve(null),
    resolveContact: (mid) =>
      resolveSenderPicture(mid, { contactIndex, senderPictureCache, getTalk }),
    avatarPathFor: (_mid, pic) => Promise.resolve(`/avatars${pic}.jpg`),
    dedupe: (_k, fn) => fn(),
    emitReady: (mid, p) => emitted.push(`${mid}:${p}`),
  });
  assertEquals(path, "/avatars/stranger.jpg");
  assertEquals(emitted, [`${sender}:/avatars/stranger.jpg`]);
  assertEquals(contactIndex.has(sender), false);
  assertEquals(senderPictureCache.get(sender), "/stranger");
  assertEquals(rpcCalls, [sender]);

  // Cache hit avoids RPC
  assertEquals(
    await resolveSenderPicture(sender, {
      contactIndex,
      senderPictureCache,
      getTalk,
    }),
    { picturePath: "/stranger" },
  );
  assertEquals(rpcCalls, [sender]);

  // Null miss is cached in senderPictureCache and not added to contactIndex
  assertEquals(
    await resolveSenderPicture(noPhotoSender, {
      contactIndex,
      senderPictureCache,
      getTalk,
    }),
    { picturePath: null },
  );
  assertEquals(
    await resolveSenderPicture(noPhotoSender, {
      contactIndex,
      senderPictureCache,
      getTalk,
    }),
    { picturePath: null },
  );
  assertEquals(contactIndex.has(noPhotoSender), false);
  assertEquals(rpcCalls, [sender, noPhotoSender]);

  // Known friend reads from contactIndex without RPC
  assertEquals(
    await resolveSenderPicture(friend, {
      contactIndex,
      senderPictureCache,
      getTalk,
    }),
    { picturePath: "/friend-pic" },
  );
  assertEquals(rpcCalls, [sender, noPhotoSender]);
});

Deno.test("handleProfileUpdate ignores strangers and updates known chats in place without promoting", async () => {
  const contactIndex = new Map<string, ContactInfo>([
    ["u-friend", {
      name: "Old Name",
      picturePath: null,
      kind: "dm",
      muted: false,
    }],
  ]);
  const senderPictureCache = new Map<string, string | null>([
    ["u-stranger", "/stale"],
  ]);
  let chatCache: ChatCache | null = {
    version: CHAT_CACHE_VERSION,
    at: 100,
    chats: [
      {
        mid: "c-group",
        name: "Group",
        kind: "group",
        avatarPath: null,
        lastActivity: 5000,
        unread: 0,
        preview: "hi",
        muted: false,
      },
      {
        mid: "u-friend",
        name: "Old Name",
        kind: "dm",
        avatarPath: null,
        lastActivity: 0,
        unread: 0,
        preview: "",
        muted: false,
      },
    ],
  };
  let savedContacts = 0;
  let savedChats = 0;
  const rpcCalls: string[] = [];
  const events: Array<{ event: string; payload?: Record<string, unknown> }> =
    [];
  const hydrated: string[] = [];
  const runtime: ContactSyncRuntime = {
    contactIndex,
    senderPictureCache,
    getTalk: () => ({
      getContact: ({ mid }) => {
        rpcCalls.push(mid);
        return Promise.resolve({
          displayName: "New Name",
          pictureStatus: "/new-pic",
          notificationDisabled: true,
        });
      },
      getChat: () => Promise.reject(new Error("unused")),
    }),
    saveContacts: () => {
      savedContacts++;
      return Promise.resolve();
    },
    getChatCache: () => chatCache,
    saveChatCache: (cache) => {
      chatCache = cache;
      savedChats++;
      return Promise.resolve();
    },
    existingAvatar: () => Promise.resolve(null),
    hydrateAvatars: (chats) => {
      hydrated.push(...chats.map((c) => c.mid));
      return Promise.resolve();
    },
    emitEvent: (event, payload) => events.push({ event, payload }),
  };

  await handleProfileUpdate("u-stranger", runtime);
  assertEquals(rpcCalls, []);
  assertEquals(contactIndex.has("u-stranger"), false);
  assertEquals(senderPictureCache.has("u-stranger"), false);
  assertEquals(events, []);
  assertEquals(savedContacts, 0);
  assertEquals(savedChats, 0);

  await handleProfileUpdate("u-friend", runtime);
  assertEquals(rpcCalls, ["u-friend"]);
  assertEquals(savedContacts, 1);
  assertEquals(savedChats, 1);
  assertEquals(contactIndex.get("u-friend")?.name, "New Name");
  assertEquals(contactIndex.get("u-friend")?.muted, true);
  assertEquals(chatCache.chats.map((c) => c.mid), ["c-group", "u-friend"]);
  assertEquals(chatCache.chats[1].name, "New Name");
  assertEquals(chatCache.chats[1].lastActivity, 0);
  assertEquals(chatCache.chats[1].muted, true);
  assertEquals(hydrated, ["u-friend"]);
  assertEquals(events, [{
    event: "chat_upsert",
    payload: { chat: chatCache.chats[1], created: false },
  }]);
});

Deno.test("handleContactAdded appends new friend with lastActivity=0 after active chats", async () => {
  const contactIndex = new Map<string, ContactInfo>();
  let chatCache: ChatCache | null = {
    version: CHAT_CACHE_VERSION,
    at: 100,
    chats: [{
      mid: "c-active",
      name: "Active Group",
      kind: "group",
      avatarPath: null,
      lastActivity: 9999,
      unread: 1,
      preview: "latest",
      muted: false,
    }],
  };
  const events: Array<{ event: string; payload?: Record<string, unknown> }> =
    [];
  const runtime: ContactSyncRuntime = {
    contactIndex,
    senderPictureCache: new Map(),
    getTalk: () => ({
      getContact: () =>
        Promise.resolve({ displayName: "New Friend", pictureStatus: "/nf" }),
      getChat: () => Promise.reject(new Error("unused")),
    }),
    saveContacts: () => Promise.resolve(),
    getChatCache: () => chatCache,
    saveChatCache: (cache) => {
      chatCache = cache;
      return Promise.resolve();
    },
    existingAvatar: () => Promise.resolve(null),
    hydrateAvatars: () => Promise.resolve(),
    emitEvent: (event, payload) => events.push({ event, payload }),
  };

  await handleContactAdded("u-new-friend", runtime);
  assertEquals(contactIndex.get("u-new-friend")?.name, "New Friend");
  assertEquals(chatCache.chats.map((c) => c.mid), ["c-active", "u-new-friend"]);
  assertEquals(chatCache.chats[1].lastActivity, 0);
  assertEquals(chatCache.chats[1].preview, "");
  assertEquals(events[0], {
    event: "chat_upsert",
    payload: { chat: chatCache.chats[1], created: true },
  });
});

Deno.test("pruneUserContacts removes non-authoritative users while keeping friends, boxes, and groups", () => {
  const contactIndex = new Map<string, ContactInfo>([
    ["u-friend", {
      name: "Friend",
      picturePath: null,
      kind: "dm",
      muted: false,
    }],
    ["u-box-bot", {
      name: "Bot",
      picturePath: null,
      kind: "bot",
      muted: false,
    }],
    ["u-stranger", {
      name: "Stranger",
      picturePath: null,
      kind: "dm",
      muted: false,
    }],
    ["c-group", {
      name: "Group",
      picturePath: null,
      kind: "group",
      muted: false,
    }],
    ["r-room", {
      name: "Room",
      picturePath: null,
      kind: "group",
      muted: false,
    }],
  ]);
  const removed = pruneUserContacts(
    contactIndex,
    new Set(["u-friend", "u-box-bot"]),
  );
  assertEquals(removed, 1);
  assertEquals([...contactIndex.keys()], [
    "u-friend",
    "u-box-bot",
    "c-group",
    "r-room",
  ]);
});

Deno.test("listChats ignores unversioned legacy chats.json and writes versioned cache", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const chatCachePath = join(directory, "chats.json");
    await Deno.writeTextFile(
      chatCachePath,
      JSON.stringify({
        at: Date.now(),
        chats: [{
          mid: "u-stale-stranger",
          name: "Stranger",
          kind: "dm",
          avatarPath: null,
          lastActivity: 999999,
          unread: 0,
          preview: "",
        }],
      }),
    );
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden");
        },
      }),
    );
    Object.defineProperty(client.base.talk, "getMessageBoxes", {
      value: () => Promise.resolve({ messageBoxes: [] }),
    });
    Object.defineProperty(client, "fetchJoinedChats", {
      value: () => Promise.resolve([]),
    });
    const events: string[] = [];
    let cached: ReturnType<CommandRuntime["getChatCache"]> = null;
    const unused = () => {
      throw new Error("Unexpected command dependency");
    };
    const runtime: CommandRuntime = {
      getClient: () => client,
      setClient: unused,
      isListening: () => false,
      setAuthDead: unused,
      getChatCache: () => cached,
      setChatCache: (value) => {
        cached = value;
      },
      msgCache: new Map(),
      contactIndex: new Map(),
      boxCursor: new Map(),
      avatarDir: directory,
      chatCachePath,
      storagePath: directory,
      lineDevice: "ANDROIDSECONDARY",
      lineVersion: "test",
      refreshCachePolicy: () => Promise.resolve(policyFor("smart")),
      cachePolicy: () => policyFor("smart"),
      dedupe: (_key, fn) => fn(),
      storeBoxCursor: () => {},
      existingFile: () => Promise.resolve(null),
      loadDiskContacts: () => Promise.resolve(),
      saveDiskContacts: () => Promise.resolve(),
      refreshContactIndex: () => Promise.resolve(),
      hydrateAvatars: () => Promise.resolve(),
      hydratePreviews: () => Promise.resolve(),
      bumpMediaEpoch: unused,
      loadDiskMessages: unused,
      saveDiskMessages: unused,
      forUiMessages: unused,
      hydrateMedia: unused,
      summarizeTalkMessage: unused,
      summarizeRawMessage: unused,
      saveAuth: unused,
      loadAuth: unused,
      loadAuthDevice: unused,
      clearAuth: unused,
      myProfilePayload: unused,
      patchE2eeGuards: unused,
      startListen: unused,
      sentMessagePayload: unused,
      touchChatPreviewFromMessage: unused,
      emitEvent: (ev) => events.push(ev),
      ok: () => {},
      fail: (_id, err) => {
        throw new Error(err);
      },
    };
    await createCommandService(runtime).listChats(1, false);
    assertEquals(events.includes("chats"), false);
    const written = JSON.parse(await Deno.readTextFile(chatCachePath));
    assertEquals(written.version, CHAT_CACHE_VERSION);
    assertEquals(written.chats, []);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("fetchMessages does not insert group BOT_CHECK senders into contactIndex", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden");
        },
      }),
    );
    Object.defineProperty(client, "getChat", {
      value: () => Promise.reject(new Error("use boxes")),
    });
    Object.defineProperty(
      client.base.talk,
      "getPreviousMessagesV2WithRequest",
      {
        value: ({ request }: { request: { messageBoxId: string } }) =>
          Promise.resolve([
            {
              id: "101",
              from: request.messageBoxId === "c-group"
                ? "u-group-bot"
                : "u-dm-bot",
              to: request.messageBoxId,
              createdTime: 1000,
              contentType: "NONE",
              contentMetadata: { BOT_CHECK: "1" },
            },
          ]),
      },
    );
    const contactIndex = new Map<string, ContactInfo>();
    const boxCursor = new Map([
      ["c-group", { messageId: 101, deliveredTime: 1000 }],
      ["u-dm-bot", { messageId: 101, deliveredTime: 1000 }],
    ]);
    const unused = () => {
      throw new Error("Unexpected command dependency");
    };
    const runtime: CommandRuntime = {
      getClient: () => client,
      setClient: unused,
      isListening: () => false,
      setAuthDead: unused,
      getChatCache: () => null,
      setChatCache: () => {},
      msgCache: new Map(),
      contactIndex,
      boxCursor,
      avatarDir: directory,
      chatCachePath: join(directory, "chats.json"),
      storagePath: directory,
      lineDevice: "ANDROIDSECONDARY",
      lineVersion: "test",
      refreshCachePolicy: () => Promise.resolve(policyFor("smart")),
      cachePolicy: () => policyFor("smart"),
      dedupe: (_key, fn) => fn(),
      storeBoxCursor: () => {},
      existingFile: () => Promise.resolve(null),
      loadDiskContacts: () => Promise.resolve(),
      saveDiskContacts: () => Promise.resolve(),
      refreshContactIndex: () => Promise.resolve(),
      hydrateAvatars: () => Promise.resolve(),
      hydratePreviews: () => Promise.resolve(),
      bumpMediaEpoch: () => 1,
      loadDiskMessages: () => Promise.resolve(null),
      saveDiskMessages: () => Promise.resolve(),
      forUiMessages: (msgs) => Promise.resolve(msgs),
      hydrateMedia: () => Promise.resolve(),
      summarizeTalkMessage: unused,
      summarizeRawMessage: (raw) =>
        Promise.resolve(raw as Record<string, unknown>),
      saveAuth: unused,
      loadAuth: unused,
      loadAuthDevice: unused,
      clearAuth: unused,
      myProfilePayload: unused,
      patchE2eeGuards: unused,
      startListen: unused,
      sentMessagePayload: unused,
      touchChatPreviewFromMessage: unused,
      emitEvent: () => {},
      ok: () => {},
      fail: (_id, err) => {
        throw new Error(err);
      },
    };
    const svc = createCommandService(runtime);
    await svc.fetchMessages(1, "c-group", 10, true, true);
    assertEquals(contactIndex.has("u-group-bot"), false);
    await svc.fetchMessages(2, "u-dm-bot", 10, true, true);
    assertEquals(contactIndex.get("u-dm-bot")?.kind, "bot");
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
