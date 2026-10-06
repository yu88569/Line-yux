import { assertEquals } from "@std/assert";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { AuthStore, parsePasswordCredentials } from "../src/auth.ts";

Deno.test("auth lifecycle uses private files", async () => {
  const directory = await Deno.makeTempDir({ prefix: "line-gtk-auth-" });
  try {
    const store = new AuthStore(directory);
    assertEquals(await store.loadToken(), null);
    await store.save("secret-token", "ANDROIDSECONDARY");
    assertEquals(await store.loadToken(), "secret-token");
    assertEquals(await store.loadDevice(), "ANDROIDSECONDARY");
    if (Deno.build.os !== "windows") {
      assertEquals((await Deno.stat(store.tokenPath)).mode! & 0o777, 0o600);
    }
    await store.clear();
    assertEquals(await store.loadToken(), null);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("password credentials validate external values without changing the password", () => {
  assertEquals(
    parsePasswordCredentials({
      email: " user@example.com ",
      password: " secret ",
    }),
    {
      email: "user@example.com",
      password: " secret ",
    },
  );
  for (
    const value of [
      null,
      {},
      { email: 42, password: "secret" },
      { email: "user@example.com", password: "" },
      { email: "bad email@example.com", password: "secret" },
      { email: "user@.example.com", password: "secret" },
      { email: "user@example.com.", password: "secret" },
      { email: "user@@example.com", password: "secret" },
    ]
  ) {
    assertEquals(parsePasswordCredentials(value), null);
  }
});

Deno.test("invalid password login never starts authentication", async () => {
  const failures: unknown[] = [];
  let authStarted = false;
  const runtime = {
    fail: (...args: unknown[]) => failures.push(args),
    setAuthDead: () => {
      authStarted = true;
    },
  } as unknown as CommandRuntime;
  const service = createCommandService(runtime);
  await service.loginPassword(12, { email: "bad", password: "secret" });
  assertEquals(authStarted, false);
  assertEquals(failures, [[12, "invalid_login_credentials"]]);
});

Deno.test("password login errors never echo upstream credentials", async () => {
  const failures: unknown[] = [];
  const runtime = {
    fail: (...args: unknown[]) => failures.push(args),
    setAuthDead: () => {
      throw new Error("user@example.com secret-password");
    },
  } as unknown as CommandRuntime;
  await createCommandService(runtime).loginPassword(13, {
    email: "user@example.com",
    password: "secret-password",
  });
  assertEquals(failures, [[13, "email_login_failed"]]);
});
