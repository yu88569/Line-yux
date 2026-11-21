import { assertEquals, assertRejects } from "@std/assert";
import { Protocols, Thrift } from "@evex/linejs/thrift";
import { loginWithAuthToken } from "../src/line_login.ts";

const token = `${btoa('{"alg":"HS256"}')}.${btoa('{"sub":"test"}')}.invalid`;
const gatewayHeaders = { server: "legy", "x-lcr": "386" };

function profileReply(): Response {
  const bytes = new Thrift().writeThrift(
    [[12, 0, [[11, 1, "u-test"]]]],
    "getProfile",
    Protocols[4],
  );
  bytes[1] = 0x41;
  return new Response(new Uint8Array(bytes).buffer);
}

Deno.test("LEGY outage retries profile over HTTPS on the same session", async () => {
  const requests: string[] = [];
  const client = await loginWithAuthToken(token, {
    device: "ANDROIDSECONDARY",
    fetch: (request) => {
      const { pathname } = new URL(request.url);
      requests.push(pathname);
      if (pathname === "/enc") {
        return Promise.resolve(
          new Response(null, {
            status: 404,
            headers: gatewayHeaders,
          }),
        );
      }
      assertEquals(new URL(request.url).protocol, "https:");
      assertEquals(request.headers.get("x-line-access"), token);
      return Promise.resolve(profileReply());
    },
  });
  assertEquals(client.base.profile?.mid, "u-test");
  await client.base.talk.getProfile();
  assertEquals(requests, ["/enc", "/S4", "/S4"]);
});

Deno.test("HTTPS-only override starts directly over HTTPS", async () => {
  const requests: string[] = [];
  const client = await loginWithAuthToken(token, {
    device: "ANDROIDSECONDARY",
    legy: { encrypted: false },
    fetch: (request) => {
      requests.push(new URL(request.url).pathname);
      return Promise.resolve(profileReply());
    },
  });
  assertEquals(client.base.profile?.mid, "u-test");
  assertEquals(requests, ["/S4"]);
});

for (
  const [name, status, headers, body] of [
    ["HTTP 403", 403, gatewayHeaders, null],
    ["an unrelated 404", 404, {}, null],
    ["a non-empty 404", 404, gatewayHeaders, "unexpected response"],
  ] as const
) {
  Deno.test(`profile does not switch transport on ${name}`, async () => {
    const requests: string[] = [];
    await assertRejects(
      () =>
        loginWithAuthToken(token, {
          device: "ANDROIDSECONDARY",
          fetch: (request) => {
            requests.push(new URL(request.url).pathname);
            return Promise.resolve(new Response(body, { status, headers }));
          },
        }),
      Error,
      body === null ? `status=${status}` : undefined,
    );
    assertEquals(requests, ["/enc"]);
  });
}

Deno.test("LEGY probe timeout retries profile over HTTPS on the same session", async () => {
  const requests: string[] = [];
  const client = await loginWithAuthToken(
    token,
    {
      device: "ANDROIDSECONDARY",
      fetch: (request) => {
        const { pathname } = new URL(request.url);
        requests.push(pathname);
        if (pathname === "/enc") {
          return new Promise((_resolve, reject) => {
            request.signal.addEventListener(
              "abort",
              () => reject(request.signal.reason),
              { once: true },
            );
          });
        }
        assertEquals(new URL(request.url).protocol, "https:");
        assertEquals(request.headers.get("x-line-access"), token);
        return Promise.resolve(profileReply());
      },
    },
    { legyProbeTimeoutMs: 20 },
  );
  assertEquals(client.base.profile?.mid, "u-test");
  await client.base.talk.getProfile();
  assertEquals(requests, ["/enc", "/S4", "/S4"]);
});
