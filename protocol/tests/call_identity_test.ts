import { assertEquals, assertRejects } from "@std/assert";
import { loadCallDeviceId } from "../src/call_identity.ts";

Deno.test("call installation identity persists privately across loads", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const first = await loadCallDeviceId(dir);
    assertEquals(atob(first).length, 32);
    assertEquals(await loadCallDeviceId(dir), first);
    if (Deno.build.os !== "windows") {
      assertEquals(
        (await Deno.stat(`${dir}/call-device-id`)).mode! & 0o777,
        0o600,
      );
    }
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("corrupt call identity is reported without silently changing it", async () => {
  const dir = await Deno.makeTempDir();
  try {
    await Deno.writeTextFile(`${dir}/call-device-id`, "corrupt");
    await assertRejects(
      () => loadCallDeviceId(dir),
      Error,
      "Invalid call device identity",
    );
    assertEquals(await Deno.readTextFile(`${dir}/call-device-id`), "corrupt");
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
