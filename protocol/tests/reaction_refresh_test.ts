import { assertEquals } from "@std/assert";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";

Deno.test("concurrent forced message refreshes respond to every request", async () => {
  const mid = "u11111111111111111111111111111111";
  const gate = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  let fetches = 0;
  const replies: unknown[] = [];
  const inflight = new Map<string, Promise<unknown>>();
  const runtime = {
    getClient: () => ({
      base: {
        talk: {
          getPreviousMessagesV2WithRequest: async () => {
            fetches++;
            started.resolve();
            await gate.promise;
            return [];
          },
        },
      },
    }),
    msgCache: new Map(),
    boxCursor: new Map([[mid, { messageId: 1, deliveredTime: 1 }]]),
    refreshCachePolicy: () => Promise.resolve({}),
    cachePolicy: () => ({ memMsg: 60000 }),
    dedupe: <T>(key: string, fn: () => Promise<T>): Promise<T> => {
      const old = inflight.get(key);
      if (old) return old as Promise<T>;
      const pending = fn().finally(() => inflight.delete(key));
      inflight.set(key, pending);
      return pending;
    },
    bumpMediaEpoch: () => 0,
    saveDiskMessages: () => Promise.resolve(),
    loadDiskMessages: () => Promise.resolve(null),
    forUiMessages: (messages: unknown) => Promise.resolve(messages),
    hydrateMedia: () => {},
    emitEvent: () => {},
    ok: (id: unknown, result: unknown) => replies.push({ id, result }),
    fail: (id: unknown, error: unknown) => replies.push({ id, error }),
  } as unknown as CommandRuntime;
  const stableClient = runtime.getClient();
  runtime.getClient = () => stableClient;
  const service = createCommandService(runtime);
  const first = service.fetchMessages(1, mid, 40, true);
  await started.promise;
  const second = service.fetchMessages(2, mid, 40, true);
  await Promise.resolve();
  assertEquals(fetches, 1);
  gate.resolve();
  await Promise.all([first, second]);
  assertEquals(fetches, 2);
  assertEquals(replies, [{ id: 1, result: { messages: [], cached: false } }, {
    id: 2,
    result: { messages: [], cached: false },
  }]);
});

Deno.test("failed background refresh reports its response without foreground progress events", async () => {
  const mid = "u11111111111111111111111111111111";
  const events: unknown[] = [];
  const errors: unknown[] = [];
  const runtime = {
    getClient: () => ({
      base: {
        talk: {
          getPreviousMessagesV2WithRequest: () =>
            Promise.reject(new Error("offline")),
        },
      },
    }),
    msgCache: new Map(),
    boxCursor: new Map([[mid, { messageId: 1, deliveredTime: 1 }]]),
    refreshCachePolicy: () => Promise.resolve({}),
    cachePolicy: () => ({ memMsg: 60000 }),
    dedupe: <T>(_key: string, fn: () => Promise<T>) => fn(),
    bumpMediaEpoch: () => 0,
    emitEvent: (name: unknown, payload: unknown) =>
      events.push({ name, payload }),
    fail: (id: unknown, error: unknown) => errors.push({ id, error }),
  } as unknown as CommandRuntime;
  await createCommandService(runtime).fetchMessages(7, mid, 40, true, true);
  assertEquals(events, []);
  assertEquals(errors, [{ id: 7, error: "offline" }]);
});
