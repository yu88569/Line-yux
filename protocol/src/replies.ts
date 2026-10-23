export type ReplyTarget = {
  chatMid: string;
  id: string;
  sender: string;
  preview: string;
};

export function parseReply(
  value: unknown,
  chatMid: string,
): ReplyTarget | undefined {
  if (value == null) return undefined;
  if (typeof value !== "object") throw new Error("invalid_reply");
  const r = value as Record<string, unknown>;
  if (
    r.chatMid !== chatMid || typeof r.id !== "string" || !/^\d+$/.test(r.id)
  ) {
    throw new Error("invalid_reply");
  }
  return {
    chatMid,
    id: r.id,
    sender: String(r.sender ?? "").slice(0, 200),
    preview: String(r.preview ?? "").slice(0, 640),
  };
}

export function incomingReply(
  raw: Record<string, unknown>,
  chatMid: string,
): ReplyTarget | undefined {
  if (
    (raw.messageRelationType !== "REPLY" && raw.messageRelationType !== 3) ||
    !raw.relatedMessageId
  ) return undefined;
  return { chatMid, id: String(raw.relatedMessageId), sender: "", preview: "" };
}
