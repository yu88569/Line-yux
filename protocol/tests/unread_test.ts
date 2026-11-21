import { assertEquals } from "@std/assert";
import { Client } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { join } from "@std/path";
import { policyFor } from "../src/cache_policy.ts";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";

function buildRuntime(directory: string, client: Client) {
  let cached: ReturnType<CommandRuntime["getChatCache"]> = {
    at: 100,
    chats: [
      {
        mid: "u-peer",
        name: "Peer",
        kind: "dm",
        avatarPath: null,
        lastActivity: 100,
        unread: 4,
        preview: "hi",
      },
    ],
  };
  let result: unknown;
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
  return {
    runtime,
    getCached: () => cached,
    getResult: () => result,
  };
}

Deno.test("listChats requests unread counts from getMessageBoxes", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden in unread test");
        },
      }),
    );
    const seenRequests: unknown[] = [];
    Object.defineProperty(client.base.talk, "getMessageBoxes", {
      value: (args: unknown) => {
        seenRequests.push(args);
        return Promise.resolve({ messageBoxes: [] });
      },
    });
    Object.defineProperty(client, "fetchJoinedChats", {
      value: () => Promise.resolve([]),
    });
    const { runtime } = buildRuntime(directory, client);
    await createCommandService(runtime).listChats(1, true);
    assertEquals(seenRequests, [{
      messageBoxListRequest: { withUnreadCount: true },
    }]);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("markRead calls sendChatChecked and persists unread=0 to chats.json", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden in unread test");
        },
      }),
    );
    const checkedCalls: unknown[] = [];
    Object.defineProperty(client.base, "getReqseq", {
      value: () => Promise.resolve(42),
    });
    Object.defineProperty(client.base.talk, "sendChatChecked", {
      value: (args: unknown) => {
        checkedCalls.push(args);
        return Promise.resolve();
      },
    });
    const { runtime, getCached, getResult } = buildRuntime(directory, client);
    await createCommandService(runtime).markRead(9, "u-peer", "123456");
    assertEquals(checkedCalls, [{
      chatMid: "u-peer",
      lastMessageId: "123456",
      seq: 42,
    }]);
    assertEquals(getResult(), {
      marked: true,
      chatMid: "u-peer",
      lastMessageId: "123456",
    });
    assertEquals(getCached()?.chats[0]?.unread, 0);
    const persisted = JSON.parse(
      await Deno.readTextFile(join(directory, "chats.json")),
    );
    assertEquals(persisted.chats[0].unread, 0);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
