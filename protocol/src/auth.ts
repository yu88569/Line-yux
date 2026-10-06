import { join } from "@std/path";
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
