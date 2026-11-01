import { assertEquals, assertThrows } from "@std/assert";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { incomingReply, parseReply } from "../src/replies.ts";

const reply = {
  chatMid: "c123",
  id: "987654321012345678",
  sender: "Alice",
  preview: "Photo",
};
Deno.test("reply boundary rejects cross-chat and temporary targets", () => {
  assertEquals(parseReply(reply, "c123"), reply);
  assertEquals(parseReply(null, "c123"), undefined);
  for (
    const value of [{ ...reply, chatMid: "c456" }, {
      ...reply,
      id: "pending-1",
    }, "987"]
  ) {
    assertThrows(() => parseReply(value, "c123"));
  }
  assertEquals(
    incomingReply(
      { relatedMessageId: reply.id, messageRelationType: 3 },
      "c123",
    )?.id,
    reply.id,
  );
  assertEquals(
    incomingReply(
      { relatedMessageId: reply.id, messageRelationType: "REPLY" },
      "c123",
    )?.id,
    reply.id,
  );
  assertEquals(
    incomingReply({
      relatedMessageId: reply.id,
      messageRelationType: "FORWARD",
    }, "c123"),
    undefined,
  );
});
Deno.test("reply preserves its relation when sent with E2EE", async () => {
  const calls: unknown[] = [];
  const outcomes: unknown[] = [];
  const runtime = {
    getClient: () => ({
      base: {
        talk: {
          sendMessage: (input: unknown) => {
            calls.push(input);
            return Promise.resolve({ id: "123" });
          },
        },
      },
    }),
    msgCache: new Map(),
    setChatCache: () => {},
    sentMessagePayload: () => ({ id: "123" }),
    touchChatPreviewFromMessage: () => {},
    ok: (_: unknown, result: unknown) => outcomes.push(result),
    fail: (_: unknown, error: unknown) => outcomes.push(error),
  } as unknown as CommandRuntime;
  await createCommandService(runtime).sendMessage(1, "c123", "answer", reply);
  assertEquals(calls, [{
    to: "c123",
    text: "answer",
    e2ee: true,
    relatedMessageId: reply.id,
  }]);
  assertEquals(outcomes, [{ message: { id: "123", reply } }]);
});
