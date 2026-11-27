import { picturePathOf } from "./contacts.ts";

export const CONTACTS_CACHE_VERSION = 2;
export const CHAT_CACHE_VERSION = 2;

export type ContactInfo = {
  name: string;
  picturePath: string | null;
  kind: string;
  muted: boolean;
};

export type ChatRow = {
  mid: string;
  name: string;
  kind: string;
  avatarPath: string | null;
  picturePath?: string | null;
  lastActivity: number;
  unread: number;
  preview: string;
  muted?: boolean;
};

export type ChatCache = {
  version?: number;
  at: number;
  chats: ChatRow[];
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

export type SenderPictureRuntime = {
  contactIndex: Map<string, ContactInfo>;
  senderPictureCache: Map<string, string | null>;
  getTalk: () => {
    getContact: (options: { mid: string }) => Promise<ContactMetadata>;
  } | null;
  maxCacheSize?: number;
};

export type ContactSyncRuntime = IdentityRuntime & {
  senderPictureCache: Map<string, string | null>;
  getChatCache: () => ChatCache | null;
  saveChatCache: (cache: ChatCache) => Promise<void>;
  existingAvatar: (mid: string) => Promise<string | null>;
  hydrateAvatars: (chats: ChatRow[]) => Promise<void>;
  emitEvent: (event: string, payload?: Record<string, unknown>) => void;
};

function contactInfoFromMetadata(
  mid: string,
  contact: ContactMetadata,
  fallbackKind: string,
): ContactInfo {
  return {
    name: contact.displayName || mid,
    picturePath: picturePathOf(contact),
    kind: /BOT/i.test(String(contact.type ?? "")) ? "bot" : fallbackKind,
    muted: !!contact.notificationDisabled,
  };
}

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
      ? contactInfoFromMetadata(mid, contact, fallback.kind)
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

export async function resolveSenderPicture(
  mid: string,
  runtime: SenderPictureRuntime,
): Promise<{ picturePath: string | null }> {
  const known = runtime.contactIndex.get(mid);
  if (known) return { picturePath: known.picturePath };
  if (runtime.senderPictureCache.has(mid)) {
    return { picturePath: runtime.senderPictureCache.get(mid) ?? null };
  }
  const talk = runtime.getTalk();
  if (!talk) return { picturePath: null };
  try {
    const contact = await talk.getContact({ mid });
    const picturePath = picturePathOf(contact);
    const maxCacheSize = runtime.maxCacheSize ?? 2048;
    if (runtime.senderPictureCache.size >= maxCacheSize) {
      const oldest = runtime.senderPictureCache.keys().next().value;
      if (oldest !== undefined) runtime.senderPictureCache.delete(oldest);
    }
    runtime.senderPictureCache.set(mid, picturePath);
    return { picturePath };
  } catch {
    return { picturePath: null };
  }
}

export function pruneUserContacts(
  contactIndex: Map<string, ContactInfo>,
  authoritativeUserMids: ReadonlySet<string>,
): number {
  let removed = 0;
  for (const mid of [...contactIndex.keys()]) {
    if (mid.startsWith("u") && !authoritativeUserMids.has(mid)) {
      contactIndex.delete(mid);
      removed++;
    }
  }
  return removed;
}

function applyContactInfoToRow(row: ChatRow, info: ContactInfo) {
  row.name = info.name || row.name;
  row.kind = info.kind || row.kind;
  row.picturePath = info.picturePath ?? row.picturePath ?? null;
  row.muted = info.muted;
}

export async function handleProfileUpdate(
  mid: string,
  runtime: ContactSyncRuntime,
): Promise<void> {
  if (!mid || !mid.startsWith("u")) return;
  runtime.senderPictureCache.delete(mid);
  const knownContact = runtime.contactIndex.has(mid);
  const chatCache = runtime.getChatCache();
  const row = chatCache?.chats.find((x) => x.mid === mid) ?? null;
  if (!knownContact && !row) return;

  const talk = runtime.getTalk();
  if (!talk) return;
  let contact: ContactMetadata;
  try {
    contact = await talk.getContact({ mid });
  } catch {
    return;
  }

  const info = contactInfoFromMetadata(
    mid,
    contact,
    runtime.contactIndex.get(mid)?.kind ?? row?.kind ?? "dm",
  );
  if (knownContact) {
    runtime.contactIndex.set(mid, info);
    await runtime.saveContacts();
  }
  if (row && chatCache) {
    applyContactInfoToRow(row, info);
    chatCache.version = CHAT_CACHE_VERSION;
    chatCache.at = Date.now();
    await runtime.saveChatCache(chatCache);
    runtime.emitEvent("chat_upsert", { chat: row, created: false });
    if (row.picturePath && !row.avatarPath) {
      void runtime.hydrateAvatars([row]);
    }
  }
}

export async function handleContactAdded(
  mid: string,
  runtime: ContactSyncRuntime,
): Promise<void> {
  if (!mid || !mid.startsWith("u")) return;
  const info = await resolveContactInfo(mid, runtime);
  const chatCache = runtime.getChatCache();
  if (!chatCache) return;

  let row = chatCache.chats.find((x) => x.mid === mid);
  const created = !row;
  if (!row) {
    row = {
      mid,
      name: info.name,
      kind: info.kind,
      picturePath: info.picturePath,
      avatarPath: await runtime.existingAvatar(mid),
      lastActivity: 0,
      unread: 0,
      preview: "",
      muted: info.muted,
    };
    chatCache.chats = [...chatCache.chats, row];
  } else {
    applyContactInfoToRow(row, info);
  }
  chatCache.version = CHAT_CACHE_VERSION;
  chatCache.at = Date.now();
  await runtime.saveChatCache(chatCache);
  runtime.emitEvent("chat_upsert", { chat: row, created });
  if (row.picturePath && !row.avatarPath) {
    void runtime.hydrateAvatars([row]);
  }
}
