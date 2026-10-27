import { assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import { reactMessage, summarizeReactions } from "../src/reactions.ts";

Deno.test("reactions preserve actor and native type in message summaries", () => {
  assertEquals(
    summarizeReactions([{
      fromUserMid: "u1",
      reactionType: { predefinedReactionType: "LOVE" },
    }]),
    [{ from: "u1", type: "LOVE" }],
  );
  assertEquals(summarizeReactions(undefined), []);
});

Deno.test("send and undo use native reaction with exact large message ID", async () => {
  const calls: unknown[] = [];
  const client = {
    base: {
      talk: {
        react: (value: unknown) => {
          calls.push(value);
          return Promise.resolve();
        },
      },
    },
  } as unknown as Client;
  await reactMessage(client, "1234567890123456789", 3);
  await reactMessage(client, "1234567890123456789", 1);
  assertEquals(calls, [{ id: 1234567890123456789n, reaction: 3 }, {
    id: 1234567890123456789n,
    reaction: 1,
  }]);
  await assertRejects(
    () => reactMessage(client, "pending-1", 3),
    Error,
    "invalid_reaction",
  );
  await assertRejects(
    () => reactMessage(client, "123", 8),
    Error,
    "invalid_reaction",
  );
  assertEquals(calls.length, 2);
});
