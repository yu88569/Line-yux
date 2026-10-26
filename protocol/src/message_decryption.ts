import type { Client } from "@evex/linejs";
import { normalizeRaw } from "./messages.ts";

export function patchMessageDecryption(e2ee: Client["base"]["e2ee"]) {
  const original = e2ee.decryptE2EEMessage.bind(e2ee);
  e2ee.decryptE2EEMessage = async (message) => {
    const normalized = { ...message, ...normalizeRaw({ ...message }) };
    try {
      const contentType: unknown = normalized.contentType;
      if (contentType === "LOCATION" || contentType === 15) {
        if (!normalized.chunks?.length) return normalized;
        // linejs's location decoder uses the wrong peer keys and string AAD.
        const envelope = await e2ee.decryptE2EEDataMessage(normalized);
        return { ...normalized, location: envelope.location };
      }
      return await original(normalized);
    } catch (error) {
      console.error("[e2ee decrypt fallback]", error);
      return normalized;
    }
  };
}
