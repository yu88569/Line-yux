import { picturePathOf } from "./contacts.ts";

export type ContactInfo = {
  name: string;
  picturePath: string | null;
  kind: string;
  muted: boolean;
};

type ContactMetadata = {
  displayName?: string;
  picturePath?: string;
  pictureStatus?: string;
  type?: string | number;
  notificationDisabled?: boolean;
};

type ChatMetadata = {
  chatName?: string;
  picturePath?: string;
  notificationDisabled?: boolean;
};

export type IdentityRuntime = {
  contactIndex: Map<string, ContactInfo>;
  getTalk: () => {
    getContact: (options: { mid: string }) => Promise<ContactMetadata>;
    getChat: (options: {
      chatMid: string;
      withMembers: boolean;
      withInvitees: boolean;
    }) => Promise<ChatMetadata>;
  } | null;
  saveContacts: () => Promise<void>;
};

export function chatInfo(mid: string, chat: ChatMetadata): ContactInfo {
  return {
    name: chat.chatName || mid,
    picturePath: picturePathOf(chat),
    kind: "group",
    muted: !!chat.notificationDisabled,
  };
}

export async function resolveContactInfo(
  mid: string,
  runtime: IdentityRuntime,
): Promise<ContactInfo> {
  const cached = runtime.contactIndex.get(mid);
  if (cached?.name && cached.name !== mid) return cached;
  const group = mid.startsWith("c") || mid.startsWith("r");
  const fallback = cached ?? {
    name: mid,
    picturePath: null,
    kind: group ? "group" : "dm",
    muted: false,
  };
  const talk = runtime.getTalk();
  if (!talk) return fallback;
  try {
    const contact = group ? null : await talk.getContact({ mid });
    const info = contact
      ? {
        name: contact.displayName || mid,
        picturePath: picturePathOf(contact),
        kind: /BOT/i.test(String(contact.type ?? "")) ? "bot" : fallback.kind,
        muted: !!contact.notificationDisabled,
      }
      : chatInfo(
        mid,
        await talk.getChat({
          chatMid: mid,
          withMembers: false,
          withInvitees: false,
        }),
      );
    runtime.contactIndex.set(mid, info);
    await runtime.saveContacts();
    return info;
  } catch {
    return fallback;
  }
}
