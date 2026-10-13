import {
  Client,
  type loginWithPassword as upstreamLoginWithPassword,
} from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";

type PasswordLoginInit = Parameters<typeof upstreamLoginWithPassword>[1];

export function createPasswordLoginBase(init: PasswordLoginInit): BaseClient {
  const base = new BaseClient(init);
  const request = base.request.request.bind(base.request);
  base.request.request = <T = unknown>(
    ...args: Parameters<typeof request>
  ): Promise<T> => {
    const [, method, protocol, , path] = args;
    // linejs 3.2.1 sends binary auth to this compact-only Android endpoint.
    if (
      path === "/api/v4p/rs" && protocol === 3 &&
      (method === "loginV2" || method === "loginZ" ||
        method === "confirmE2EELogin")
    ) {
      args[2] = 4;
    }
    return request<T>(...args);
  };
  return base;
}

/** Mirror the pinned factory while correcting only password auth's wire format. */
export async function loginWithPassword(
  ...[options, init]: Parameters<typeof upstreamLoginWithPassword>
): ReturnType<typeof upstreamLoginWithPassword> {
  const base = createPasswordLoginBase(init);
  base.on("pincall", options.onPincodeRequest);
  await base.loginProcess.withPassword({
    email: options.email,
    password: options.password,
    pincode: options.pincode,
  });
  await base.loginProcess.ready();
  return new Client(base);
}
