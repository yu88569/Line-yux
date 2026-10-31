import { assertEquals } from "@std/assert";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { historyCursor, mergeHistory } from "../src/history.ts";
const mid = "u11111111111111111111111111111111";
const cursor = (id: number) => ({
  messageId: String(id),
  deliveredTime: String(id),
});
const raw = (id: number) => ({
  id: String(id),
  deliveredTime: id,
  createdTime: id,
  text: String(id),
});
function fixture() {
  const replies: {
    id: unknown;
    result?: Record<string, unknown>;
    error?: string;
  }[] = [];
  let page = [raw(100), raw(99)];
  const calls: unknown[] = [];
  const client = {
    base: {
      talk: {
        getPreviousMessagesV2WithRequest: (args: unknown) => {
          calls.push(args);
          return Promise.resolve(page);
        },
      },
    },
  };
  let current = client;
  const cache = new Map();
  let saved: Record<string, unknown>[] = [];
  let beforeSave = async (_messages: Record<string, unknown>[]) => {};
  const runtime = {
    getClient: () => current,
    msgCache: cache,
    boxCursor: new Map([[mid, { messageId: 100, deliveredTime: 100 }]]),
    refreshCachePolicy: () => Promise.resolve({}),
    cachePolicy: () => ({ memMsg: 0 }),
    dedupe: <T>(_key: string, fn: () => Promise<T>) => fn(),
    bumpMediaEpoch: () => 0,
    loadDiskMessages: () => Promise.resolve(saved),
    saveDiskMessages: async (_mid: string, m: Record<string, unknown>[]) => {
      await beforeSave(m);
      saved = m;
    },
    forUiMessages: (m: unknown) => Promise.resolve(m),
    hydrateMedia: () => Promise.resolve(),
    emitEvent: () => {},
    summarizeRawMessage: (m: ReturnType<typeof raw>) =>
      Promise.resolve({ ...m, historyCursor: historyCursor(m) }),
    ok: (id: unknown, result: Record<string, unknown>) =>
      replies.push({ id, result }),
    fail: (id: unknown, error: string) => replies.push({ id, error }),
  } as unknown as CommandRuntime;
  return {
    service: createCommandService(runtime),
    runtime,
    replies,
    calls,
    cache,
    setPage: (value: typeof page) => {
      page = value;
    },
    saved: () => saved,
    clearDisk: () => {
      saved = [];
    },
    onSave: (hook: typeof beforeSave) => {
      beforeSave = hook;
    },
    changeClient: () => {
      current = { ...client };
    },
  };
}
Deno.test("history validates exact string cursors without losing 64-bit IDs", () => {
  assertEquals(
    historyCursor({ id: 9223372036854775807n, deliveredTime: 100n }),
    { messageId: "9223372036854775807", deliveredTime: "100" },
  );
  assertEquals(historyCursor({ messageId: "bad", deliveredTime: "100" }), null);
  assertEquals(
    mergeHistory([{ id: "1", createdTime: 1, filePath: "/download" }], [{
      id: "1",
      createdTime: 1,
      filePath: null,
      text: "new",
    }]),
    [{ id: "1", createdTime: 1, filePath: "/download", text: "new" }],
  );
});
Deno.test("older pages request41, exclude inclusive boundary, merge cache and stop at EOF", async () => {
  const f = fixture();
  f.setPage(Array.from({ length: 41 }, (_, i) => raw(100 - i)));
  await f.service.fetchOlderMessages(1, mid, cursor(100));
  assertEquals(f.calls[0], {
    request: {
      messageBoxId: mid,
      endMessageId: { messageId: 100n, deliveredTime: 100n },
      messagesCount: 41,
    },
  });
  assertEquals((f.replies[0].result?.messages as unknown[]).length, 40);
  assertEquals(f.replies[0].result?.before, cursor(60));
  assertEquals(f.replies[0].result?.hasMore, true);
  f.setPage([raw(60), raw(59)]);
  await f.service.fetchOlderMessages(2, mid, cursor(60));
  assertEquals(f.replies[1].result?.hasMore, false);
  assertEquals(f.saved().length, 41);
  f.setPage([raw(59)]);
  await f.service.fetchOlderMessages(3, mid, cursor(59));
  assertEquals(f.replies[2].result?.messages, []);
  assertEquals(f.replies[2].result?.hasMore, false);
});
Deno.test("latest refresh retains loaded history after ordinary cache invalidation", async () => {
  const f = fixture();
  f.setPage([raw(60), raw(59)]);
  await f.service.fetchOlderMessages(1, mid, cursor(60));
  f.cache.delete(mid);
  f.setPage([raw(100), raw(99)]);
  await f.service.fetchMessages(2, mid, 40, true);
  assertEquals(f.saved().map((m) => m.id), ["59", "99", "100"]);
  await f.service.clearHistory();
  f.cache.clear();
  f.clearDisk();
  f.setPage([raw(101)]);
  await f.service.fetchMessages(3, mid, 40, true);
  assertEquals(f.saved().map((m) => m.id), ["101"]);
});
Deno.test("old-session and cache-cleared in-flight pages cannot write cache", async () => {
  for (const invalidate of ["session", "cache"]) {
    const f = fixture();
    const gate = Promise.withResolvers<ReturnType<typeof raw>[]>();
    const client = f.runtime.getClient();
    if (!client) throw Error("fixture client");
    client.base.talk.getPreviousMessagesV2WithRequest = () =>
      gate.promise as ReturnType<
        typeof client.base.talk.getPreviousMessagesV2WithRequest
      >;
    const task = f.service.fetchOlderMessages(1, mid, cursor(100));
    if (invalidate === "session") f.changeClient();
    else f.service.clearHistory();
    gate.resolve([raw(100), raw(99)]);
    await task;
    assertEquals(f.replies[0].error, "session_changed");
    assertEquals(f.saved(), []);
  }
});

Deno.test("concurrent latest and older persistence cannot overwrite newer history", async () => {
  const f = fixture();
  await f.service.fetchMessages(1, mid, 40, true);
  const gate = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  f.onSave(async (messages) => {
    if (
      messages.some((m) => m.id === "101") &&
      !messages.some((m) => m.id === "98")
    ) {
      started.resolve();
      await gate.promise;
    }
  });
  f.setPage([raw(101), raw(100)]);
  const latest = f.service.fetchMessages(2, mid, 40, true);
  await started.promise;
  f.setPage([raw(99), raw(98)]);
  const older = f.service.fetchOlderMessages(3, mid, cursor(99));
  await Promise.resolve();
  gate.resolve();
  await Promise.all([latest, older]);
  assertEquals(f.saved().map((m) => m.id), ["98", "99", "100", "101"]);
});
