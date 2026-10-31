type Json = Record<string, unknown>;
export type HistoryCursor = { messageId: string; deliveredTime: string };

export function historyCursor(value: unknown): HistoryCursor | null {
  if (!value || typeof value !== "object") return null;
  const id = "messageId" in value
    ? String(value.messageId)
    : "id" in value
    ? String(value.id)
    : "";
  const time = "deliveredTime" in value ? String(value.deliveredTime) : "";
  if (!/^\d+$/.test(id) || !/^\d+$/.test(time)) return null;
  return { messageId: id, deliveredTime: time };
}

export function mergeHistory(previous: Json[], incoming: Json[]): Json[] {
  const messages = new Map(
    previous.map((message) => [String(message.id), message]),
  );
  for (const message of incoming) {
    const old = messages.get(String(message.id));
    const merged = { ...old, ...message };
    for (const key of ["imagePath", "audioPath", "filePath"]) {
      if (!merged[key] && old?.[key]) merged[key] = old[key];
    }
    messages.set(String(message.id), merged);
  }
  return [...messages.values()].sort((a, b) =>
    Number(a.createdTime) - Number(b.createdTime) ||
    String(a.id).localeCompare(String(b.id))
  );
}
