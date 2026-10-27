import type { Client } from "@evex/linejs";

export function summarizeReactions(value: unknown) {
  if (!Array.isArray(value)) return [];
  return value.filter((item) => item && typeof item === "object").map((
    item,
  ) => ({
    from: typeof item.fromUserMid === "string" ? item.fromUserMid : "",
    type: item.reactionType?.predefinedReactionType ?? 0,
  }));
}

export async function reactMessage(
  client: Pick<Client, "base">,
  messageId: string,
  value: unknown,
) {
  const reaction = Number(value);
  if (
    !/^[0-9]+$/.test(messageId) || !Number.isInteger(reaction) ||
    reaction < 1 || reaction > 7
  ) {
    throw new Error("invalid_reaction");
  }
  await client.base.talk.react({
    id: BigInt(messageId),
    reaction: reaction as 1 | 2 | 3 | 4 | 5 | 6 | 7,
  });
}
