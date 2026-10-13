import { join } from "@std/path";
import { InternalError } from "@evex/linejs/base";
import { enums } from "@evex/linejs-types";
import { readPrivateTextFile, writePrivateTextFile } from "./storage.ts";

export type LineDevice = "ANDROID" | "ANDROIDSECONDARY" | "DESKTOPWIN";

export class AuthStore {
  readonly tokenPath: string;
  readonly devicePath: string;

  constructor(dataDir: string) {
    this.tokenPath = join(dataDir, "auth-token.txt");
    this.devicePath = join(dataDir, "auth-device.txt");
  }

  async save(token: string, device: LineDevice): Promise<void> {
    await writePrivateTextFile(this.tokenPath, token);
    await writePrivateTextFile(this.devicePath, device);
  }

  async loadToken(): Promise<string | null> {
    try {
      const token = (await readPrivateTextFile(this.tokenPath)).trim();
      return token || null;
    } catch {
      return null;
    }
  }

  async loadDevice(): Promise<LineDevice> {
    try {
      const device = (await readPrivateTextFile(this.devicePath)).trim();
      if (
        device === "ANDROID" || device === "ANDROIDSECONDARY" ||
        device === "DESKTOPWIN"
      ) {
        return device;
      }
    } catch { /* missing */ }
    return "DESKTOPWIN";
  }

  async clear(): Promise<void> {
    for (const path of [this.tokenPath, this.devicePath]) {
      try {
        await Deno.remove(path);
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error;
      }
    }
  }
}

export type PasswordCredentials = { email: string; password: string };

const passwordLoginMessages = {
  validation: "email_login_format_unsupported",
  network: "email_login_network_failed",
  login_disallowed: "email_login_disallowed",
  invalid_credentials: "email_login_rejected",
  rate_limited: "email_login_rate_limited",
  unknown: "email_login_failed",
} as const;

const passwordLoginErrorNames = [
  "Error",
  "TypeError",
  "RangeError",
  "SyntaxError",
  "InternalError",
  "RequestError",
  "RegExpUnmatch",
  "TimeoutError",
  "ClientClosed",
  "NotAuthorized",
] as const;
const passwordLoginMethods = [
  "getRSAKeyInfo",
  "loginV2",
  "loginZ",
  "getProfile",
] as const;
const upstreamLoginSources = new Set([
  "base/request/mod.ts",
  "base/request/legy.ts",
  "base/login/mod.ts",
  "base/login/rsa-verify.ts",
  "base/e2ee/mod.ts",
  "base/core/mod.ts",
  "base/storage/file.ts",
  "base/thrift/mod.ts",
  "base/thrift/rename/parser.ts",
  "client/login.ts",
  "client/client.ts",
]);
const localLoginSources = new Set([
  "auth.ts",
  "commands.ts",
  "main.ts",
  "storage.ts",
]);

type UnknownPasswordLoginDetails = {
  errorName: typeof passwordLoginErrorNames[number] | null;
  requestMethod: typeof passwordLoginMethods[number] | null;
  httpStatus: number | null;
  sources: string[];
};

export type PasswordLoginDiagnostic =
  & {
    serverCode: number | null;
  }
  & (
    | { kind: Exclude<keyof typeof passwordLoginMessages, "unknown"> }
    | ({ kind: "unknown" } & UnknownPasswordLoginDetails)
  );

function unknownPasswordLoginDetails(
  error: unknown,
): UnknownPasswordLoginDetails {
  const details: UnknownPasswordLoginDetails = {
    errorName: null,
    requestMethod: null,
    httpStatus: null,
    sources: [],
  };
  if (!(error instanceof Error)) return details;
  details.errorName =
    passwordLoginErrorNames.find((name) => name === error.name) ?? null;
  details.requestMethod =
    passwordLoginMethods.find((method) =>
      error.message.startsWith(`Request internal failed, ${method}(`)
    ) ?? null;
  const status = /^Request internal failed: status=([1-5]\d{2}) headers=/.exec(
    error.message,
  );
  if (status) details.httpStatus = Number(status[1]);
  // A stack's first line contains the raw error. Only emit known module paths.
  for (const frame of error.stack?.split("\n").slice(1) ?? []) {
    if (!/^\s+at\s/.test(frame)) continue;
    const upstream =
      /(?:\(|\s)https:\/\/jsr\.io\/@evex\/linejs\/3\.2\.1\/([a-zA-Z0-9_/-]+\.ts):([1-9]\d{0,5}):([1-9]\d{0,5})\)?$/
        .exec(frame);
    const local =
      /(?:\(|\s)file:\/\/\/[^()\s?#]*\/protocol\/src\/([a-z_]+\.ts):([1-9]\d{0,5}):([1-9]\d{0,5})\)?$/
        .exec(frame);
    let source: string | null = null;
    if (upstream && upstreamLoginSources.has(upstream[1])) {
      source = `linejs/${upstream[1]}:${upstream[2]}:${upstream[3]}`;
    } else if (local && localLoginSources.has(local[1])) {
      source = `protocol/src/${local[1]}:${local[2]}:${local[3]}`;
    }
    if (source && !details.sources.includes(source)) {
      details.sources.push(source);
    }
    if (details.sources.length === 6) break;
  }
  return details;
}

/** Raw upstream text must never leave this allowlisted diagnostic boundary. */
export function classifyPasswordLoginFailure(error: unknown):
  & PasswordLoginDiagnostic
  & {
    messageKey:
      typeof passwordLoginMessages[keyof typeof passwordLoginMessages];
  } {
  let kind: keyof typeof passwordLoginMessages = "unknown";
  let serverCode: number | null = null;
  if (error instanceof InternalError) {
    if (error.type === "RegExpUnmatch") {
      kind = "validation";
    } else if (error.type === "TimeoutError") {
      kind = "network";
    } else if (error.type === "RequestError") {
      const code: unknown = error.data.code;
      if (
        typeof code === "number" && Number.isInteger(code) &&
        code >= 0 && code <= 0x7fffffff
      ) {
        serverCode = code;
      } else if (typeof code === "string") {
        serverCode = Object.entries(enums.ErrorCode).find(([name]) =>
          name === code
        )?.[1] ?? null;
      }
      // linejs 3.2.1 decodes TalkException.code as an ErrorCode name.
      switch (serverCode) {
        case enums.ErrorCode.INVALID_IDENTITY_CREDENTIAL:
          kind = "invalid_credentials";
          break;
        case enums.ErrorCode.NOT_ALLOWED_SECONDARY_DEVICE:
          kind = "login_disallowed";
          break;
        case enums.ErrorCode.EXCESSIVE_ACCESS:
          kind = "rate_limited";
          break;
        case enums.ErrorCode.INVALID_PASSWORD_FORMAT:
          kind = "validation";
          break;
      }
    }
  } else if (
    error instanceof Deno.errors.ConnectionRefused ||
    error instanceof Deno.errors.ConnectionReset ||
    error instanceof Deno.errors.ConnectionAborted ||
    error instanceof Deno.errors.NotConnected ||
    error instanceof Deno.errors.TimedOut ||
    (error instanceof DOMException &&
      (error.name === "TimeoutError" || error.name === "AbortError")) ||
    (error instanceof TypeError &&
      (error.message.startsWith("error sending request for url (") ||
        error.message === "Failed to fetch" ||
        error.message === "fetch failed"))
  ) {
    kind = "network";
  }
  if (kind === "unknown") {
    return {
      kind,
      serverCode,
      messageKey: passwordLoginMessages[kind],
      ...unknownPasswordLoginDetails(error),
    };
  }
  return { kind, serverCode, messageKey: passwordLoginMessages[kind] };
}

export function parsePasswordCredentials(
  value: unknown,
): PasswordCredentials | null {
  if (!value || typeof value !== "object") return null;
  const { email, password } = value as Record<string, unknown>;
  if (typeof email !== "string" || typeof password !== "string") return null;
  const normalized = email.trim();
  const domain = normalized.split("@")[1] ?? "";
  if (
    !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalized) || password.length === 0 ||
    domain.startsWith(".") || domain.endsWith(".")
  ) {
    return null;
  }
  return { email: normalized, password };
}
