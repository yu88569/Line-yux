import { join } from "@std/path";
import { readPrivateTextFile, writePrivateTextFile } from "./storage.ts";

export async function loadCallDeviceId(dataDir: string): Promise<string> {
  const path = join(dataDir, "call-device-id");
  try {
    const value = await readPrivateTextFile(path);
    if (!/^[A-Za-z0-9+/]{43}=$/.test(value) || btoa(atob(value)) !== value) {
      throw new Error("Invalid call device identity");
    }
    return value;
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
  const id = btoa(
    String.fromCharCode(...crypto.getRandomValues(new Uint8Array(32))),
  );
  await writePrivateTextFile(path, id);
  return id;
}
