import { assertEquals } from "@std/assert";
import { Client } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { join } from "@std/path";
import { policyFor } from "../src/cache_policy.ts";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import {
  chatInfo,
  type ContactInfo,
  type IdentityRuntime,
  resolveContactInfo,
} from "../src/chat_identity.ts";

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
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
