/**
 * Vendored @evex/linejs call media plane with local patches for 1:1 T103.
 * Upstream 3.2.1 derives DATA SRTP but never uses it in decrypt (PT98 fail).
 * Incoming audio and release lifecycle ported from evex-dev/linejs commits
 * f01397b, ef97dc8, and f55d36ebc9bac947ddb8ff867a27293c608db57b.
 */
export {
  PlanetTransport,
  type PlanetTransportOpts,
  type PlanetIncomingIdentity,
  type PlanetEndReason,
} from "./planet/transport.ts";
export { opusCodecFactory } from "./opus.ts";
export { type IncomingAudioCall, parseIncomingAudioCall } from "./incoming.ts";
