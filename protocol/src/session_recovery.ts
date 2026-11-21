import { InternalError } from "@evex/linejs/base";
import { enums } from "@evex/linejs-types";

export type SessionErrorContext = "restore" | "runtime";

export type SessionErrorClassification =
  | { readonly kind: "revoked"; readonly code: string }
  | { readonly kind: "transient"; readonly reason: string }
  | { readonly kind: "unknown"; readonly reason: string };

export type RestoreResult<T> =
  | { readonly kind: "restored"; readonly value: T }
  | { readonly kind: "revoked"; readonly error: string }
  | { readonly kind: "recoverable_failed"; readonly error: string }
  | { readonly kind: "aborted" };

export const DEFAULT_RESTORE_BACKOFF_MS = [
  2_000,
  5_000,
  10_000,
  20_000,
  30_000,
] as const;

export const MAX_UNKNOWN_RESTORE_ATTEMPTS = 3;

const ALWAYS_REVOKED_CODES = new Set([
  "NOT_AUTHORIZED_DEVICE",
  "NOT_AUTHORIZED_SESSION",
  "NOT_AVAILABLE_SESSION",
  "NOT_AUTHENTICATED",
  "V3_TOKEN_CLIENT_LOGGED_OUT",
  "CLIENT_LOGGED_OUT",
]);

const CORE_SESSION_AUTH_CODES = new Set([
  "AUTHENTICATION_FAILED",
  "MUST_REFRESH_V3_TOKEN",
]);

const REFRESH_REVOKED_CODES = new Set([
  "ALREADY_EXPIRED",
  "INVALID_STATE",
]);

const TRANSIENT_SERVER_CODES = new Set([
  "SERVER_BUSY",
  "SHOULD_RETRY",
  "CONGESTION_CONTROL",
  "MAINTENANCE_ERROR",
  "INTERNAL_ERROR",
  "SYSTEM_ERROR",
  "DB_FAILED",
  "TALK_PROXY_EXCEPTION",
  "EXCESSIVE_ACCESS",
]);

const CORE_SESSION_PATHS = [
  "(/S4)",
  "(/S3)",
  "(/SYNC4)",
  "(/SYNC3)",
  "(/AS4)",
  "(/EXT/auth/tokenrefresh/v1)",
] as const;

const REVOKED_TEXT_TOKEN =
  /\b(NOT_AUTHORIZED_DEVICE|NOT_AUTHORIZED_SESSION|NOT_AVAILABLE_SESSION|NOT_AUTHENTICATED|V3_TOKEN_CLIENT_LOGGED_OUT|CLIENT_LOGGED_OUT)\b/;

const HTTP_STATUS_RE = /^Request internal failed: status=(\d{3})\b/;

const TRANSIENT_MESSAGE_RE =
  /(error sending request for url \(|dns error|Temporary failure in name resolution|operation was aborted due to timeout|timed out|\b(?:ECONNRESET|ECONNREFUSED|ENOTFOUND|EAI_AGAIN|EHOSTUNREACH|ENETUNREACH)\b)/i;

const errorCodeNameByNumber = new Map<number, string>(
  Object.entries(enums.ErrorCode).map(([name, code]) => [code, name]),
);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function messageOf(value: unknown): string {
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value ?? "") || String(value ?? "");
  } catch {
    return String(value ?? "");
  }
}

export function errorSummary(error: unknown): string {
  const base = messageOf(error).trim();
  const name = error instanceof Error && error.name ? error.name : "";
  const primary = base
    ? (name && name !== "Error" && !base.startsWith(name)
      ? `${name}: ${base}`
      : base)
    : (name || "unknown error");
  if (isRecord(error) && "cause" in error && error.cause !== undefined) {
    const causeMsg = messageOf(error.cause).trim();
    if (causeMsg && !primary.includes(causeMsg)) {
      return `${primary}: ${causeMsg}`;
    }
  }
  return primary;
}

function errorChain(error: unknown): unknown[] {
  const chain: unknown[] = [];
  const seen = new Set<unknown>();
  let current: unknown = error;
  for (
    let depth = 0;
    depth < 4 && current !== undefined && current !== null;
    depth++
  ) {
    if (typeof current === "object") {
      if (seen.has(current)) break;
      seen.add(current);
    }
    chain.push(current);
    current = isRecord(current) && "cause" in current
      ? current.cause
      : undefined;
  }
  return chain;
}

function resolveErrorCodeName(rawCode: unknown): string | null {
  if (typeof rawCode === "string" && rawCode.length > 0) return rawCode;
  if (typeof rawCode === "number" && Number.isInteger(rawCode)) {
    return errorCodeNameByNumber.get(rawCode) ?? null;
  }
  return null;
}

function extractRequestErrorCode(data: unknown): string | null {
  if (!isRecord(data)) return null;
  const direct = resolveErrorCodeName(data.code);
  if (direct) return direct;
  if (isRecord(data.e)) {
    return resolveErrorCodeName(data.e.code);
  }
  return null;
}

function isCoreSessionEndpointMessage(message: string): boolean {
  return CORE_SESSION_PATHS.some((path) => message.includes(path));
}

function classifySingleItem(
  item: unknown,
  context: SessionErrorContext,
): SessionErrorClassification | null {
  if (item instanceof InternalError) {
    if (item.type === "NotAuthorized" || item.type === "RefreshError") {
      return { kind: "revoked", code: item.type };
    }
    if (item.type === "RequestError") {
      const codeName = extractRequestErrorCode(item.data);
      if (codeName) {
        if (ALWAYS_REVOKED_CODES.has(codeName)) {
          return { kind: "revoked", code: codeName };
        }
        if (
          CORE_SESSION_AUTH_CODES.has(codeName) &&
          (context === "restore" || isCoreSessionEndpointMessage(item.message))
        ) {
          return { kind: "revoked", code: codeName };
        }
        if (
          REFRESH_REVOKED_CODES.has(codeName) &&
          item.message.includes("(/EXT/auth/tokenrefresh/v1)")
        ) {
          return { kind: "revoked", code: codeName };
        }
        if (TRANSIENT_SERVER_CODES.has(codeName)) {
          return { kind: "transient", reason: codeName };
        }
      }
    }
  }

  const text = messageOf(item);
  const revokedMatch = REVOKED_TEXT_TOKEN.exec(text);
  if (revokedMatch) {
    return { kind: "revoked", code: revokedMatch[1] };
  }

  if (item instanceof InternalError) {
    if (item.type === "TimeoutError" || item.type === "ClientClosed") {
      return { kind: "transient", reason: item.type };
    }
  }

  if (
    item instanceof Error &&
    (item.name === "TimeoutError" || item.name === "AbortError")
  ) {
    return { kind: "transient", reason: item.name };
  }

  if (
    item instanceof Deno.errors.ConnectionRefused ||
    item instanceof Deno.errors.ConnectionReset ||
    item instanceof Deno.errors.ConnectionAborted ||
    item instanceof Deno.errors.NotConnected ||
    item instanceof Deno.errors.TimedOut ||
    item instanceof Deno.errors.BrokenPipe ||
    item instanceof Deno.errors.UnexpectedEof ||
    item instanceof Deno.errors.AddrNotAvailable
  ) {
    return { kind: "transient", reason: item.name };
  }

  if (
    item instanceof TypeError &&
    (item.message === "fetch failed" ||
      item.message === "Failed to fetch" ||
      item.message.startsWith("error sending request for url (") ||
      /\b(dns error|socket)\b/i.test(item.message))
  ) {
    return { kind: "transient", reason: item.message };
  }

  if (item instanceof Error) {
    const statusMatch = HTTP_STATUS_RE.exec(item.message);
    if (statusMatch) {
      const status = Number(statusMatch[1]);
      if (status === 408 || status === 429 || status >= 500) {
        return { kind: "transient", reason: `http_${status}` };
      }
    }
    if (TRANSIENT_MESSAGE_RE.test(item.message)) {
      return { kind: "transient", reason: item.message };
    }
  }

  return null;
}

export function classifySessionError(
  error: unknown,
  context: SessionErrorContext,
): SessionErrorClassification {
  const chain = errorChain(error);
  for (const item of chain) {
    const classified = classifySingleItem(item, context);
    if (classified?.kind === "revoked") return classified;
  }
  for (const item of chain) {
    const classified = classifySingleItem(item, context);
    if (classified?.kind === "transient") return classified;
  }
  return { kind: "unknown", reason: errorSummary(error) };
}

export function abortableSleep(
  ms: number,
  signal?: AbortSignal,
): Promise<void> {
  if (signal?.aborted) {
    return Promise.reject(new DOMException("Aborted", "AbortError"));
  }
  return new Promise<void>((resolve, reject) => {
    const onAbort = () => {
      cleanup();
      reject(new DOMException("Aborted", "AbortError"));
    };
    const timer = setTimeout(() => {
      cleanup();
      resolve();
    }, ms);
    const cleanup = () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

export async function restoreSessionWithRetry<T>(options: {
  attempt: () => Promise<T>;
  onRetry: (
    info: { attempt: number; retryInMs: number; error: string },
  ) => void;
  signal?: AbortSignal;
  backoffMs?: readonly number[];
  maxUnknownAttempts?: number;
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
}): Promise<RestoreResult<T>> {
  const backoff = options.backoffMs?.length
    ? options.backoffMs
    : DEFAULT_RESTORE_BACKOFF_MS;
  const maxUnknownAttempts = options.maxUnknownAttempts ??
    MAX_UNKNOWN_RESTORE_ATTEMPTS;
  const sleep = options.sleep ?? abortableSleep;

  let totalAttempts = 0;
  let unknownFailures = 0;

  while (true) {
    if (options.signal?.aborted) {
      return { kind: "aborted" };
    }
    totalAttempts += 1;
    try {
      const value = await options.attempt();
      if (options.signal?.aborted) {
        return { kind: "aborted" };
      }
      return { kind: "restored", value };
    } catch (error) {
      if (options.signal?.aborted) {
        return { kind: "aborted" };
      }
      const summary = errorSummary(error);
      const classification = classifySessionError(error, "restore");
      switch (classification.kind) {
        case "revoked":
          return { kind: "revoked", error: summary };
        case "unknown": {
          unknownFailures += 1;
          if (unknownFailures >= maxUnknownAttempts) {
            return { kind: "recoverable_failed", error: summary };
          }
          break;
        }
        case "transient":
          break;
        default: {
          const _exhaustive: never = classification;
          return _exhaustive;
        }
      }
      const retryInMs =
        backoff[Math.min(totalAttempts - 1, backoff.length - 1)];
      options.onRetry({ attempt: totalAttempts, retryInMs, error: summary });
      try {
        await sleep(retryInMs, options.signal);
      } catch {
        return { kind: "aborted" };
      }
    }
  }
}

export type TokenObservableClient = {
  base: {
    authToken?: string;
    on: (
      event: "update:authtoken",
      listener: (token: string) => void,
    ) => unknown;
    off: (
      event: "update:authtoken",
      listener: (token: string) => void,
    ) => unknown;
  };
};

export function bindAuthTokenPersistence(
  client: TokenObservableClient,
  options: {
    initialToken?: string | null;
    isActive: () => boolean;
    save: (token: string) => Promise<void>;
    onError?: (error: unknown) => void;
  },
): () => void {
  let disposed = false;
  let lastSaved = options.initialToken?.trim() ?? "";
  let queue: Promise<void> = Promise.resolve();

  const onTokenUpdate = (rawToken: string) => {
    const next = rawToken.trim();
    if (disposed || !options.isActive() || !next) return;
    if (client.base.authToken !== next) {
      client.base.authToken = next;
    }
    if (next === lastSaved) return;
    lastSaved = next;
    queue = queue.then(async () => {
      if (disposed || !options.isActive()) return;
      await options.save(next);
    }).catch((error) => {
      options.onError?.(error);
    });
  };

  client.base.on("update:authtoken", onTokenUpdate);
  return () => {
    if (disposed) return;
    disposed = true;
    client.base.off("update:authtoken", onTokenUpdate);
  };
}
