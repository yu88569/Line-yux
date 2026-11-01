import { assertEquals } from "@std/assert";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";

Deno.test("text sends with E2EE and reports encryption failure without fallback", async () => {
  const calls: unknown[] = [];
  const failures: unknown[] = [];
  const runtime = {
    getClient: () => ({
      base: {
        talk: {
          sendMessage: (options: unknown) => {
            calls.push(options);
            return Promise.reject(new Error("encryption failed"));
          },
        },
      },
      getChat: () => {
        throw new Error("unexpected fallback");
      },
    }),
    fail: (_id: unknown, error: unknown) => failures.push(error),
  } as unknown as CommandRuntime;
  await createCommandService(runtime).sendMessage(1, "chat", "hello");
  assertEquals(calls, [{ to: "chat", text: "hello", e2ee: true }]);
  assertEquals(failures, ["encryption failed"]);
});
