import {
  Client,
  type loginWithAuthToken as upstreamLoginWithAuthToken,
  type loginWithQR as upstreamLoginWithQR,
} from "@evex/linejs";
import { BaseClient, InternalError } from "@evex/linejs/base";

export const DEFAULT_LEGY_PROBE_TIMEOUT_MS = 10_000;

export type LineLoginOptions = {
  legyProbeTimeoutMs?: number;
};

type LoginOptions = ConstructorParameters<typeof BaseClient>[0];

class LegyGatewayUnavailable extends Error {}

export function createLineLoginBase(
  init: LoginOptions,
  options?: LineLoginOptions,
): BaseClient {
  const gateway = new URL(
    init.legy?.endpoint ?? "https://gf.line.naver.jp/enc",
  );
  const probeTimeoutMs = options?.legyProbeTimeoutMs ??
    DEFAULT_LEGY_PROBE_TIMEOUT_MS;
  let checkingProfile = false;
  const base = new BaseClient({
    ...init,
    fetch: async (request) => {
      const fetcher = init.fetch ?? globalThis.fetch;
      const isLegyProbe = checkingProfile &&
        request.url === gateway.href &&
        gateway.protocol === "https:" && request.headers.has("x-lcs");
      let response: Response;
      if (isLegyProbe && probeTimeoutMs > 0) {
        const controller = new AbortController();
        const outerSignal = request.signal;
        const onOuterAbort = () => controller.abort(outerSignal.reason);
        if (outerSignal.aborted) {
          controller.abort(outerSignal.reason);
        } else {
          outerSignal.addEventListener("abort", onOuterAbort, { once: true });
        }
        let probeTimedOut = false;
        const timer = setTimeout(() => {
          probeTimedOut = true;
          controller.abort(
            new DOMException("LINE LEGY gateway timed out", "TimeoutError"),
          );
        }, probeTimeoutMs);
        try {
          response = await fetcher(
            new Request(request, { signal: controller.signal }),
          );
        } catch (error) {
          if (
            probeTimedOut ||
            (error instanceof Error && error.name === "TimeoutError") ||
            (error instanceof InternalError && error.type === "TimeoutError")
          ) {
            throw new LegyGatewayUnavailable("LINE LEGY gateway timed out");
          }
          throw error;
        } finally {
          clearTimeout(timer);
          outerSignal.removeEventListener("abort", onOuterAbort);
        }
      } else {
        response = await fetcher(request);
      }
      if (
        isLegyProbe &&
        !response.redirected && response.status === 404 &&
        response.headers.get("server") === "legy" &&
        response.headers.get("x-lcr") === "386" &&
        (await response.clone().arrayBuffer()).byteLength === 0
      ) {
        throw new LegyGatewayUnavailable("LINE LEGY gateway returned HTTP 404");
      }
      return response;
    },
  });
  base.on("update:authtoken", (nextToken) => {
    const trimmed = nextToken.trim();
    if (trimmed) base.authToken = trimmed;
  });
  // In linejs 3.2.1, ready() reads getProfile before emitting the ready event.
  const ready = base.loginProcess.ready.bind(base.loginProcess);
  base.loginProcess.ready = async () => {
    checkingProfile = base.legy.encrypted !== false;
    try {
      await ready();
    } catch (error) {
      if (!(error instanceof LegyGatewayUnavailable)) throw error;
      base.legy.encrypted = false;
      checkingProfile = false;
      await ready();
      console.warn(
        "[auth] LEGY unavailable; using direct HTTPS for this session",
      );
    } finally {
      checkingProfile = false;
    }
  };
  return base;
}

export async function loginWithAuthToken(
  authToken: Parameters<typeof upstreamLoginWithAuthToken>[0],
  init: Parameters<typeof upstreamLoginWithAuthToken>[1],
  options?: LineLoginOptions,
): Promise<Client> {
  if (!authToken) throw new InternalError("NotAuthorized", "try login first");
  const base = createLineLoginBase(init, options);
  await base.loginProcess.login({ authToken });
  return new Client(base);
}

export async function loginWithQR(
  options: Parameters<typeof upstreamLoginWithQR>[0],
  init: Parameters<typeof upstreamLoginWithQR>[1],
  loginOptions?: LineLoginOptions,
): Promise<Client> {
  const base = createLineLoginBase(init, loginOptions);
  base.on("qrcall", options.onReceiveQRUrl);
  base.on("pincall", options.onPincodeRequest);
  await base.loginProcess.withQrCode({});
  await base.loginProcess.ready();
  return new Client(base);
}
