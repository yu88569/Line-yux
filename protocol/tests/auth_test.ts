import { assertEquals } from "@std/assert";
import { InternalError } from "@evex/linejs/base";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import {
  AuthStore,
  classifyPasswordLoginFailure,
  parsePasswordCredentials,
} from "../src/auth.ts";

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

for (
  const [code, key, kind, serverCode] of [
    [
      "INVALID_IDENTITY_CREDENTIAL",
      "email_login_rejected",
      "invalid_credentials",
      18,
    ],
    [18, "email_login_rejected", "invalid_credentials", 18],
    [
      "NOT_ALLOWED_SECONDARY_DEVICE",
      "email_login_disallowed",
      "login_disallowed",
      44,
    ],
    [44, "email_login_disallowed", "login_disallowed", 44],
    ["EXCESSIVE_ACCESS", "email_login_rate_limited", "rate_limited", 4],
    [4, "email_login_rate_limited", "rate_limited", 4],
    [
      "INVALID_PASSWORD_FORMAT",
      "email_login_format_unsupported",
      "validation",
      69,
    ],
    [69, "email_login_format_unsupported", "validation", 69],
    [555, "email_login_failed", "unknown", 555],
    [
      "user@example.com secret-password token PIN=123456",
      "email_login_failed",
      "unknown",
      null,
    ],
    [123456789012345, "email_login_failed", "unknown", null],
  ] as const
) {
  Deno.test(`password login safely reports upstream code ${typeof code === "number" ? code : serverCode ?? "unknown"}`, async () => {
    const failures: unknown[] = [];
    const diagnostics: unknown[] = [];
    const runtime = {
      fail: (...args: unknown[]) => failures.push(args),
      reportPasswordLoginFailure: (diagnostic: unknown) =>
        diagnostics.push(diagnostic),
      setAuthDead: () => {
        const error = new InternalError(
          "RequestError",
          "user@example.com secret-password token PIN=123456 https://example.com/private",
          {
            code,
            reason: "user@example.com secret-password token PIN=123456",
          },
        );
        error.stack =
          "RequestError: user@example.com secret-password token PIN=123456";
        throw error;
      },
    } as unknown as CommandRuntime;
    await createCommandService(runtime).loginPassword(14, {
      email: "user@example.com",
      password: "secret-password",
    });
    assertEquals(failures, [[14, key]]);
    assertEquals(diagnostics, [{
      kind,
      serverCode,
      ...(kind === "unknown"
        ? {
          errorName: "RequestError",
          requestMethod: null,
          httpStatus: null,
          sources: [],
        }
        : {}),
    }]);
  });
}

Deno.test("password login reports a safe network failure", async () => {
  const failures: unknown[] = [];
  const diagnostics: unknown[] = [];
  const runtime = {
    fail: (...args: unknown[]) => failures.push(args),
    reportPasswordLoginFailure: (diagnostic: unknown) =>
      diagnostics.push(diagnostic),
    setAuthDead: () => {
      throw new Deno.errors.ConnectionRefused(
        "user@example.com secret-password",
      );
    },
  } as unknown as CommandRuntime;
  await createCommandService(runtime).loginPassword(15, {
    email: "user@example.com",
    password: "secret-password",
  });
  assertEquals(failures, [[15, "email_login_network_failed"]]);
  assertEquals(diagnostics, [{ kind: "network", serverCode: null }]);
});

Deno.test("real upstream password syntax rejection is actionable without network", async () => {
  const directory = await Deno.makeTempDir({ prefix: "line-gtk-login-test-" });
  const failures: unknown[] = [];
  const diagnostics: unknown[] = [];
  const originalFetch = globalThis.fetch;
  let networkAttempts = 0;
  globalThis.fetch = () => {
    networkAttempts++;
    throw new Error("Network must never be used in this test");
  };
  try {
    const runtime = {
      fail: (...args: unknown[]) => failures.push(args),
      reportPasswordLoginFailure: (diagnostic: unknown) =>
        diagnostics.push(diagnostic),
      setAuthDead: () => {},
      storagePath: `${directory}/storage.json`,
      lineDevice: "ANDROIDSECONDARY",
      lineVersion: "26.6.2",
    } as unknown as CommandRuntime;
    await createCommandService(runtime).loginPassword(16, {
      email: "user@example.com",
      password: " space-containing-password ",
    });
    assertEquals(networkAttempts, 0);
    assertEquals(failures, [[16, "email_login_format_unsupported"]]);
    assertEquals(diagnostics, [{ kind: "validation", serverCode: null }]);
  } finally {
    globalThis.fetch = originalFetch;
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("unknown upstream HTTP errors expose only status and allowed source positions", () => {
  const error = new Error(
    'Request internal failed: status=403 headers={"Authorization":"secret-token"} body=<user@example.com password PIN=123456>',
  );
  error.stack = [
    "Error: user@example.com secret-token PIN=123456 https://private.example.com/login",
    "    at secret-password (https://jsr.io/@evex/linejs/3.2.1/base/request/mod.ts:186:10)",
    "    at doLoginPassword (file:///home/user@example.com/private/protocol/src/commands.ts:233:15)",
    "    at secret (https://jsr.io/@evex/linejs/3.2.1/base/secret-token.ts:123456:10)",
    "    at secret (https://jsr.io/@evex/linejs/3.2.1/base/login/mod.ts?token=secret-token:99:2)",
    "    at secret (https://jsr.io/@evex/linejs/3.2.2/base/login/mod.ts:99:2)",
    "    at secret (https://private.example.com/linejs/3.2.1/base/login/mod.ts:99:2)",
  ].join("\n");
  assertEquals<unknown>(classifyPasswordLoginFailure(error), {
    kind: "unknown",
    serverCode: null,
    messageKey: "email_login_failed",
    errorName: "Error",
    requestMethod: null,
    httpStatus: 403,
    sources: [
      "linejs/base/request/mod.ts:186:10",
      "protocol/src/commands.ts:233:15",
    ],
  });
});

Deno.test("unknown RequestError method survives command diagnostics without raw Thrift data", async () => {
  const failures: unknown[] = [];
  const diagnostics: unknown[] = [];
  const error = new InternalError(
    "RequestError",
    'Request internal failed, loginV2(/api/v4p/rs) -> {"1":{"1":18,"2":"user@example.com secret-password token PIN=123456"}}',
    {
      1: { 1: 18, 2: "user@example.com secret-password token PIN=123456" },
    },
  );
  error.stack =
    "RequestError: user@example.com secret-password token PIN=123456\n    at LoginClient.loginV2 (https://jsr.io/@evex/linejs/3.2.1/base/login/mod.ts:677:36)";
  const runtime = {
    fail: (...args: unknown[]) => failures.push(args),
    reportPasswordLoginFailure: (diagnostic: unknown) =>
      diagnostics.push(diagnostic),
    setAuthDead: () => {
      throw error;
    },
  } as unknown as CommandRuntime;
  await createCommandService(runtime).loginPassword(17, {
    email: "user@example.com",
    password: "secret-password",
  });
  assertEquals(failures, [[17, "email_login_failed"]]);
  assertEquals(diagnostics, [{
    kind: "unknown",
    serverCode: null,
    errorName: "RequestError",
    requestMethod: "loginV2",
    httpStatus: null,
    sources: ["linejs/base/login/mod.ts:677:36"],
  }]);
});

Deno.test("unknown diagnostic fields require exact allowlisted names and upstream prefixes", () => {
  const message =
    "secret-password Request internal failed: status=401 loginV2(token)";
  for (
    const [error, errorName] of [
      [new TypeError(message), "TypeError"],
      [new RangeError(message), "RangeError"],
      [new SyntaxError(message), "SyntaxError"],
      [new InternalError("NotAuthorized", message), "NotAuthorized"],
      [new InternalError("secret-token PIN=123456", message), null],
    ] as const
  ) {
    error.stack =
      "Error: https://jsr.io/@evex/linejs/3.2.1/base/login/mod.ts:123:45";
    assertEquals<unknown>(classifyPasswordLoginFailure(error), {
      kind: "unknown",
      serverCode: null,
      messageKey: "email_login_failed",
      errorName,
      requestMethod: null,
      httpStatus: null,
      sources: [],
    });
  }
});

for (const method of ["getRSAKeyInfo", "loginV2", "loginZ", "getProfile"]) {
  Deno.test(`unknown diagnostic allows request method ${method}`, () => {
    const error = new Error(
      `Request internal failed, ${method}(/private-secret-token) -> user@example.com password`,
    );
    error.stack = "Error: secret-token";
    assertEquals<unknown>(classifyPasswordLoginFailure(error), {
      kind: "unknown",
      serverCode: null,
      messageKey: "email_login_failed",
      errorName: "Error",
      requestMethod: method,
      httpStatus: null,
      sources: [],
    });
  });
}
