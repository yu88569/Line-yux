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
for (const successAt of [0, 1, 2, 3]) {
  Deno.test(`reply native relation survives fallback ${successAt}`, async () => {
    const calls: Record<string, unknown>[] = [];
    const outcomes: unknown[] = [];
    const sender = (input: Record<string, unknown>) => {
      const attempt = calls.length;
      calls.push(input);
      if (attempt < successAt) return Promise.reject(Error("failed"));
      return Promise.resolve({ id: "123", raw: { id: "123" } });
    };
    const runtime = {
      getClient: () => ({
        base: { talk: { sendMessage: sender } },
        getChat: () => Promise.resolve({ sendMessage: sender }),
      }),
      msgCache: new Map(),
      setChatCache: () => {},
      summarizeTalkMessage: () => Promise.resolve({ id: "123" }),
      sentMessagePayload: () => ({ id: "123" }),
      touchChatPreviewFromMessage: () => {},
      ok: (_: unknown, result: unknown) => outcomes.push(result),
      fail: (_: unknown, error: unknown) => outcomes.push(error),
    } as unknown as CommandRuntime;
    await createCommandService(runtime).sendMessage(1, "c123", "answer", reply);
    assertEquals(calls.length, Math.min(successAt + 1, 3));
    for (const call of calls) {
      assertEquals(call.relatedMessageId, reply.id);
      assertEquals(call.text, "answer");
    }
    if (successAt < 3) {
      assertEquals(
        (outcomes[0] as { message: { reply: unknown } }).message.reply,
        reply,
      );
    } else assertEquals(outcomes, ["failed"]);
  });
}
Deno.test("reply persists when chat summary falls back", async () => {
  let result: unknown;
  const runtime = {
    getClient: () => ({
      base: { talk: { sendMessage: () => Promise.reject(Error("base")) } },
      getChat: () =>
        Promise.resolve({
          sendMessage: () => Promise.resolve({ raw: { id: "123" } }),
        }),
    }),
    msgCache: new Map(),
    setChatCache: () => {},
    summarizeTalkMessage: () => Promise.reject(Error("summary")),
    sentMessagePayload: () => ({ id: "123" }),
    touchChatPreviewFromMessage: () => {},
    ok: (_: unknown, r: unknown) => result = r,
    fail: () => {},
  } as unknown as CommandRuntime;
  await createCommandService(runtime).sendMessage(1, "c123", "answer", reply);
  assertEquals(
    (result as { message: { reply: unknown } }).message.reply,
    reply,
  );
});
