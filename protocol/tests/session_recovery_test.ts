import { assert, assertEquals } from "@std/assert";
import { BaseClient, InternalError } from "@evex/linejs/base";
import {
  bindAuthTokenPersistence,
  classifySessionError,
  errorSummary,
  restoreSessionWithRetry,
} from "../src/session_recovery.ts";

Deno.test("classifySessionError marks timeout, DNS, reset, and busy server errors as transient", () => {
  const timeout = new DOMException(
    "The operation was aborted due to timeout",
    "TimeoutError",
  );
  assertEquals(classifySessionError(timeout, "runtime").kind, "transient");
  assertEquals(classifySessionError(timeout, "restore").kind, "transient");

  const dnsCause = new Error(
    "error sending request for url (https://gf.line.naver.jp/enc): client error (Connect): dns error: failed to lookup address information: Temporary failure in name resolution",
  );
  const fetchFailed = new TypeError("fetch failed", { cause: dnsCause });
  assertEquals(classifySessionError(fetchFailed, "restore").kind, "transient");
  assert(
    errorSummary(fetchFailed).includes("Temporary failure in name resolution"),
  );

  for (
    const err of [
      new Deno.errors.ConnectionReset("reset"),
      new Deno.errors.TimedOut("timed out"),
      new InternalError("TimeoutError", "request timed out"),
      new InternalError(
        "RequestError",
        "Request internal failed, sync(/SYNC4)",
        {
          code: "SERVER_BUSY",
        },
      ),
      new Error("Request internal failed: status=503 headers=[] body=<>"),
    ]
  ) {
    assertEquals(classifySessionError(err, "restore").kind, "transient");
    assertEquals(classifySessionError(err, "runtime").kind, "transient");
  }
});

Deno.test("classifySessionError distinguishes revoked sessions from non-core auth errors", () => {
  for (
    const err of [
      new InternalError("NotAuthorized", "try login first"),
      new InternalError("RefreshError", "refreshToken not found"),
      new InternalError(
        "RequestError",
        "Request internal failed, getProfile(/S4)",
        { code: "NOT_AUTHORIZED_DEVICE" },
      ),
      new InternalError(
        "RequestError",
        "Request internal failed, getProfile(/S4)",
        { e: { code: 14 } },
      ),
      new Error("V3_TOKEN_CLIENT_LOGGED_OUT"),
    ]
  ) {
    assertEquals(classifySessionError(err, "restore").kind, "revoked");
    assertEquals(classifySessionError(err, "runtime").kind, "revoked");
  }

  const coreAuthFailed = new InternalError(
    "RequestError",
    "Request internal failed, getProfile(/S4)",
    { code: "AUTHENTICATION_FAILED" },
  );
  assertEquals(classifySessionError(coreAuthFailed, "restore").kind, "revoked");
  assertEquals(classifySessionError(coreAuthFailed, "runtime").kind, "revoked");

  const channelAuthFailed = new InternalError(
    "RequestError",
    "Request internal failed, getChannelToken(/CH4)",
    { code: "AUTHENTICATION_FAILED" },
  );
  assertEquals(
    classifySessionError(channelAuthFailed, "runtime").kind,
    "unknown",
  );

  // Bare "AUTHENTICATION" text without a core session error code must not wipe auth.
  assertEquals(
    classifySessionError(
      new Error("AUTHENTICATION temporarily unavailable"),
      "runtime",
    ).kind,
    "unknown",
  );
});

Deno.test("restoreSessionWithRetry recovers after transient timeout and DNS errors", async () => {
  let attempts = 0;
  const retries: { attempt: number; retryInMs: number; error: string }[] = [];
  const delays: number[] = [];
  const result = await restoreSessionWithRetry({
    backoffMs: [2_000, 5_000, 10_000],
    attempt: () => {
      attempts += 1;
      if (attempts === 1) {
        return Promise.reject(
          new DOMException(
            "The operation was aborted due to timeout",
            "TimeoutError",
          ),
        );
      }
      if (attempts === 2) {
        return Promise.reject(
          new TypeError("fetch failed", {
            cause: new Error(
              "error sending request for url (https://gf.line.naver.jp/enc): dns error: Temporary failure in name resolution",
            ),
          }),
        );
      }
      return Promise.resolve("restored-client");
    },
    onRetry: (info) => retries.push(info),
    sleep: (ms) => {
      delays.push(ms);
      return Promise.resolve();
    },
  });

  assertEquals(result, { kind: "restored", value: "restored-client" });
  assertEquals(attempts, 3);
  assertEquals(delays, [2_000, 5_000]);
  assertEquals(retries.length, 2);
  assertEquals(retries[0].attempt, 1);
  assertEquals(retries[1].attempt, 2);
});

Deno.test("restoreSessionWithRetry stops immediately on revoked and caps unknown errors", async () => {
  let revokedAttempts = 0;
  const revokedRetries: number[] = [];
  const revokedResult = await restoreSessionWithRetry({
    attempt: () => {
      revokedAttempts += 1;
      return Promise.reject(
        new InternalError(
          "RequestError",
          "Request internal failed, getProfile(/S4)",
          { code: "NOT_AUTHORIZED_DEVICE" },
        ),
      );
    },
    onRetry: ({ attempt }) => revokedRetries.push(attempt),
    sleep: () => Promise.resolve(),
  });
  assertEquals(revokedAttempts, 1);
  assertEquals(revokedRetries, []);
  assertEquals(revokedResult.kind, "revoked");

  let unknownAttempts = 0;
  const unknownRetries: number[] = [];
  const unknownResult = await restoreSessionWithRetry({
    maxUnknownAttempts: 3,
    backoffMs: [100, 200],
    attempt: () => {
      unknownAttempts += 1;
      return Promise.reject(new Error("unexpected thrift frame"));
    },
    onRetry: ({ attempt }) => unknownRetries.push(attempt),
    sleep: () => Promise.resolve(),
  });
  assertEquals(unknownAttempts, 3);
  assertEquals(unknownRetries, [1, 2]);
  assertEquals(unknownResult, {
    kind: "recoverable_failed",
    error: "unexpected thrift frame",
  });
});

Deno.test("restoreSessionWithRetry aborts cleanly during backoff sleep without leaking timers", async () => {
  const controller = new AbortController();
  let attempts = 0;
  const promise = restoreSessionWithRetry({
    signal: controller.signal,
    backoffMs: [60_000],
    attempt: () => {
      attempts += 1;
      return Promise.reject(
        new DOMException(
          "The operation was aborted due to timeout",
          "TimeoutError",
        ),
      );
    },
    onRetry: () => {
      controller.abort();
    },
  });
  assertEquals(await promise, { kind: "aborted" });
  assertEquals(attempts, 1);
});

Deno.test("bindAuthTokenPersistence persists refreshed tokens in order and stops when disposed", async () => {
  const base = new BaseClient({ device: "ANDROIDSECONDARY" });
  base.authToken = "token-1";
  let active = true;
  const saved: string[] = [];
  let releaseFirst!: () => void;
  const firstPending = new Promise<void>((resolve) => {
    releaseFirst = resolve;
  });

  const dispose = bindAuthTokenPersistence(
    { base },
    {
      initialToken: "token-1",
      isActive: () => active,
      save: async (token) => {
        if (token === "token-2") await firstPending;
        saved.push(token);
      },
    },
  );

  // Duplicate of initial token is ignored.
  base.emit("update:authtoken", "token-1");
  // Concurrent updates serialize cleanly and update base.authToken immediately.
  base.emit("update:authtoken", "token-2");
  base.emit("update:authtoken", "token-3");
  assertEquals(base.authToken, "token-3");
  releaseFirst();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(saved, ["token-2", "token-3"]);
  assertEquals(base.listeners.get("update:authtoken")?.length ?? 0, 1);

  // Inactive or disposed emitter must not overwrite storage.
  active = false;
  base.emit("update:authtoken", "token-4");
  active = true;
  dispose();
  assertEquals(base.listeners.get("update:authtoken")?.length ?? 0, 0);
  base.emit("update:authtoken", "token-5");
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(saved, ["token-2", "token-3"]);
});
