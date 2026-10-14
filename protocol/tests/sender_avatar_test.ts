import { assertEquals } from "@std/assert";
import { resolveSenderAvatar } from "../src/sender_avatar.ts";
const sender = `u${"1".repeat(32)}`;
function fixture() {
  const calls: string[] = [];
  const emitted: string[] = [];
  const pending = new Map<string, Promise<unknown>>();
  return {
    calls,
    emitted,
    runtime: {
      existingPath: (_mid: string): Promise<string | null> =>
        Promise.resolve(null),
      resolveContact: (mid: string) => {
        calls.push(mid);
        return Promise.resolve({ picturePath: "sender-photo" });
      },
      avatarPathFor: (mid: string, picture: string) => {
        calls.push(`${mid}/${picture}`);
        return Promise.resolve("/cache/sender.jpg");
      },
      dedupe: <T>(key: string, fn: () => Promise<T>): Promise<T> => {
        const existing = pending.get(key);
        if (existing) return existing as Promise<T>;
        const result = fn().finally(() => pending.delete(key));
        pending.set(key, result);
        return result;
      },
      emitReady: (mid: string, path: string) => emitted.push(`${mid}:${path}`),
    },
  };
}
Deno.test("sender avatar rejects invalid and group MID before lookup", async () => {
  const f = fixture();
  for (const invalid of ["../secret", `c${"1".repeat(32)}`, "", null]) {
    assertEquals(await resolveSenderAvatar(invalid, f.runtime), null);
  }
  assertEquals(f.calls, []);
  assertEquals(f.emitted, []);
});
Deno.test("cached sender avatar avoids contact lookup", async () => {
  const f = fixture();
  f.runtime.existingPath = () => Promise.resolve("/cache/existing.jpg");
  assertEquals(
    await resolveSenderAvatar(sender, f.runtime),
    "/cache/existing.jpg",
  );
  assertEquals(f.calls, []);
  assertEquals(f.emitted, [`${sender}:/cache/existing.jpg`]);
});
Deno.test("concurrent nonfriend rows resolve once with sender identity", async () => {
  const f = fixture();
  assertEquals(
    await Promise.all([
      resolveSenderAvatar(sender, f.runtime),
      resolveSenderAvatar(sender, f.runtime),
    ]),
    ["/cache/sender.jpg", "/cache/sender.jpg"],
  );
  assertEquals(f.calls, [sender, `${sender}/sender-photo`]);
  assertEquals(f.emitted, [`${sender}:/cache/sender.jpg`]);
});
Deno.test("missing photo and contact failure quietly preserve fallback", async () => {
  const f = fixture();
  f.runtime.resolveContact = () => Promise.resolve({ picturePath: "" });
  assertEquals(await resolveSenderAvatar(sender, f.runtime), null);
  f.runtime.resolveContact = () => Promise.reject(new Error("offline"));
  assertEquals(await resolveSenderAvatar(sender, f.runtime), null);
  assertEquals(f.emitted, []);
});
