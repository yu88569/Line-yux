import { assertEquals, assertRejects } from "@std/assert";
import { InternalError } from "@evex/linejs/base";
import { type NestedArray, Protocols, Thrift } from "@evex/linejs/thrift";
import { Buffer } from "node:buffer";
import { classifyPasswordLoginFailure } from "../src/auth.ts";
import {
  createPasswordLoginBase,
  loginWithPassword,
} from "../src/password_login.ts";

const thrift = new Thrift();

function reply(method: string, protocol: 3 | 4, value: NestedArray): Response {
  const bytes = thrift.writeThrift(value, method, Protocols[protocol]);
  // writeThrift emits a CALL header; a service response is a REPLY.
  bytes[protocol === 3 ? 3 : 1] = protocol === 3 ? 2 : 0x41;
  return new Response(new Uint8Array(bytes).buffer);
}

function rsaReply(): Response {
  return reply("getRSAKeyInfo", 3, [[12, 0, [
    [11, 1, "test-key"],
    [11, 2, "f".repeat(256)],
    [11, 3, "10001"],
    [11, 4, "test-session"],
  ]]]);
}

function rejectedReply(method: string, protocol: 3 | 4): Response {
  return reply(method, protocol, [[12, 1, [
    [8, 1, 18],
    [11, 2, "test-only rejected credentials"],
  ]]]);
}

for (const device of ["ANDROID", "ANDROIDSECONDARY", "DESKTOPWIN"] as const) {
  Deno.test(`${device} password login uses the endpoint's protocol and parses a rejection`, async () => {
    const requests: { path: string; marker: number }[] = [];
    let failure: unknown;
    try {
      await loginWithPassword({
        email: "test@example.com",
        password: "fake-password",
        onPincodeRequest: () => {},
      }, {
        device,
        fetch: async (input) => {
          const request = new Request(input);
          const path = new URL(request.url).pathname;
          const bytes = new Uint8Array(await request.arrayBuffer());
          requests.push({ path, marker: bytes[0] });
          if (path === "/api/v3/TalkService.do") return rsaReply();
          if (path === "/api/v4p/rs") {
            return bytes[0] === 0x82
              ? rejectedReply("loginV2", 4)
              : new Response();
          }
          if (path === "/api/v3p/rs") return rejectedReply("loginZ", 3);
          throw new Error("Unexpected test request");
        },
      });
    } catch (error) {
      failure = error;
    }
    assertEquals(requests, [
      { path: "/api/v3/TalkService.do", marker: 0x80 },
      {
        path: device === "DESKTOPWIN" ? "/api/v3p/rs" : "/api/v4p/rs",
        marker: device === "DESKTOPWIN" ? 0x80 : 0x82,
      },
    ]);
    assertEquals(classifyPasswordLoginFailure(failure), {
      kind: "invalid_credentials",
      serverCode: 18,
      messageKey: "email_login_rejected",
    });
  });
}

Deno.test("compact password login response reaches the PIN callback", async () => {
  const pins: string[] = [];
  const stopAfterPin = new Error("Test stops before PIN confirmation");
  await assertRejects(
    () =>
      loginWithPassword({
        email: "test@example.com",
        password: "fake-password",
        onPincodeRequest: (pin) => {
          pins.push(pin);
        },
      }, {
        device: "ANDROIDSECONDARY",
        fetch: async (input) => {
          const request = new Request(input);
          const path = new URL(request.url).pathname;
          if (path === "/api/v3/TalkService.do") return rsaReply();
          if (path === "/api/v4p/rs") {
            const bytes = new Uint8Array(await request.arrayBuffer());
            return bytes[0] === 0x82
              ? reply("loginV2", 4, [[12, 0, [[11, 3, "fake-verifier"]]]])
              : new Response();
          }
          if (path === "/LF1") throw stopAfterPin;
          throw new Error("Unexpected test request");
        },
      }),
    Error,
    stopAfterPin.message,
  );
  assertEquals(pins, ["114514"]);
});

Deno.test("Android email E2EE confirmation uses compact protocol and parses its response", async () => {
  const markers: number[] = [];
  const base = createPasswordLoginBase({
    device: "ANDROIDSECONDARY",
    fetch: async (input) => {
      const request = new Request(input);
      assertEquals(new URL(request.url).pathname, "/api/v4p/rs");
      const bytes = new Uint8Array(await request.arrayBuffer());
      markers.push(bytes[0]);
      return bytes[0] === 0x82
        ? reply("confirmE2EELogin", 4, [[11, 0, "confirmed-test-verifier"]])
        : new Response();
    },
  });
  let result: unknown;
  try {
    result = await base.loginProcess.confirmE2EELogin(
      "fake-verifier",
      Buffer.from("fake-device-secret"),
    );
  } catch (error) {
    result = error instanceof InternalError ? error.type : "unparsed_response";
  }
  assertEquals(markers, [0x82]);
  assertEquals(result, "confirmed-test-verifier");
});

Deno.test("Android legacy email loginZ uses compact auth and preserves binary RSA", async () => {
  const requests: { path: string; marker: number }[] = [];
  const base = createPasswordLoginBase({
    device: "ANDROIDSECONDARY",
    fetch: async (input) => {
      const request = new Request(input);
      const path = new URL(request.url).pathname;
      const bytes = new Uint8Array(await request.arrayBuffer());
      requests.push({ path, marker: bytes[0] });
      if (path === "/api/v3/TalkService.do") return rsaReply();
      if (path === "/api/v4p/rs") return rejectedReply("loginZ", 4);
      throw new Error("Unexpected test request");
    },
  });
  const error = await assertRejects(() =>
    base.loginProcess.requestEmailLogin(
      "test@example.com",
      "fake-password",
      "123456",
      false,
    ), InternalError);
  assertEquals(error.data.code, "INVALID_IDENTITY_CREDENTIAL");
  assertEquals(requests, [
    { path: "/api/v3/TalkService.do", marker: 0x80 },
    { path: "/api/v4p/rs", marker: 0x82 },
  ]);
});

for (
  const [method, path, protocol, marker] of [
    ["getRSAKeyInfo", "/api/v4p/rs", 3, 0x80],
    ["loginV2", "/S4", 3, 0x80],
    ["loginV2", "/api/v4p/rs", 4, 0x82],
  ] as const
) {
  Deno.test(`password adapter preserves explicit protocol ${protocol} for ${method} at ${path}`, async () => {
    const base = createPasswordLoginBase({
      device: "ANDROIDSECONDARY",
      fetch: async (input) => {
        const request = new Request(input);
        assertEquals(new URL(request.url).pathname, path);
        const bytes = new Uint8Array(await request.arrayBuffer());
        assertEquals(bytes[0], marker);
        return reply(method, protocol, [[11, 0, "test-result"]]);
      },
    });
    assertEquals(
      await base.request.request<string>([], method, protocol, false, path),
      "test-result",
    );
  });
}
