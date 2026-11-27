import type { Client, TalkMessage } from "@evex/linejs";
import * as calls from "./calls.ts";
import { messageChatMid } from "./messages.ts";

type Json = Record<string, unknown>;

type ListenerScheduler = {
  now: () => number;
  every: (callback: () => void, interval: number) => () => void;
};

const scheduler: ListenerScheduler = {
  now: () => performance.now(),
  every: (callback, interval) => {
    const timer = setInterval(callback, interval);
    return () => clearInterval(timer);
  },
};

export type ListenerRuntime = {
  getClient: () => Client | null;
  getMyMid?: () => string;
  onChatRead?: (chatMid: string, messageId: string) => void | Promise<void>;
  onTransportStalled: () => void;
  scheduler?: ListenerScheduler;
  setListening: (listening: boolean) => void;
  invalidateMessages: (chatMid: string) => void;
  summarizeTalkMessage: (
    message: TalkMessage,
    opts?: { withMedia?: boolean },
  ) => Promise<Json>;
  storeBoxCursor: (
    mid: string,
    value?: { messageId?: unknown; deliveredTime?: unknown } | null,
  ) => void;
  upsertChatFromMessage: (message: Json) => Promise<void>;
  hydrateLiveMedia: (
    message: TalkMessage,
    summary: Json,
    chatMid: string,
  ) => Promise<void>;
  onContactAdded?: (mid: string) => void | Promise<void>;
  onProfileUpdated?: (mid: string) => void | Promise<void>;
  emitEvent: (event: string, payload?: Json) => void;
};

export function createListener(runtime: ListenerRuntime) {
  const {
    summarizeTalkMessage,
    storeBoxCursor,
    upsertChatFromMessage,
    hydrateLiveMedia,
    emitEvent,
  } = runtime;

  let active: { client: Client; dispose: () => void } | null = null;

  function stop() {
    const session = active;
    active = null;
    if (!session) return;
    session.dispose();
    runtime.setListening(false);
  }

  function startListen() {
    const client = runtime.getClient();
    if (client && active?.client === client) return;
    stop();
    if (!client) return;
    const clock = runtime.scheduler ?? scheduler;
    const controller = new AbortController();
    const session = { client, dispose: () => {} };
    active = session;
    const ownsSession = () =>
      active === session && runtime.getClient() === client;
    let lastProgressAt = clock.now();
    const sentMessageIds = new Set<string>();
    const rememberSentMessageId = (rawId: unknown) => {
      if (rawId == null) return;
      const id = String(rawId);
      if (!id) return;
      if (sentMessageIds.has(id)) sentMessageIds.delete(id);
      else if (sentMessageIds.size >= 256) {
        const oldest = sentMessageIds.values().next().value;
        if (oldest !== undefined) sentMessageIds.delete(oldest);
      }
      sentMessageIds.add(id);
    };
    runtime.setListening(true);

    // Register BEFORE listen() so early ops are not missed.
    const onMessage = async (msg: TalkMessage) => {
      if (!ownsSession()) return;
      try {
        const rawId = String(
          (msg.raw as { id?: unknown } | undefined)?.id ?? "",
        );
        const fromSendOp = !!rawId && sentMessageIds.delete(rawId);
        const message = await summarizeTalkMessage(msg, { withMedia: false });
        if (!ownsSession()) return;
        if (fromSendOp) {
          message.mine = true;
          if (!message.from) {
            const selfMid = runtime.getMyMid?.() ?? "";
            if (selfMid) message.from = selfMid;
          }
        }
        const peer = messageChatMid(message);
        if (
          fromSendOp && message.reply && typeof message.reply === "object" &&
          peer
        ) {
          (message.reply as Json).chatMid = peer;
        }
        runtime.invalidateMessages(peer);
        // Advance box cursor so later refetch can see this message.
        storeBoxCursor(peer, {
          messageId: message.id,
          deliveredTime: message.createdTime,
        });
        emitEvent("message", { message: { ...message, imagePath: null } });
        await upsertChatFromMessage(message);
        if (ownsSession() && message.needsMedia) {
          // Use the live TalkMessage.getData path — history refetch often misses
          // brand-new messages and left bubbles on "Image unavailable" until restart.
          void hydrateLiveMedia(msg, message, peer);
        }
      } catch (e) {
        console.error("[listen message]", e);
      }
    };

    const onEvent = (op: {
      type?: string | number;
      param1?: string;
      param2?: string;
      param3?: string;
      message?: { id?: string | number; from?: string; to?: string };
    }) => {
      if (!ownsSession()) return;
      try {
        const type = String(op?.type ?? "");
        if (type === "SEND_MESSAGE" || type === "25") {
          rememberSentMessageId(op.message?.id);
        } else if (type === "SEND_CHAT_CHECKED" || type === "40") {
          const chatMid = String(op.param1 ?? "");
          const messageId = String(op.param2 ?? "");
          if (chatMid) {
            void Promise.resolve(runtime.onChatRead?.(chatMid, messageId))
              .catch(
                (error) => console.error("[listen chat_read]", chatMid, error),
              );
            emitEvent("chat_read", { chatMid, messageId });
          }
        } else if (
          type === "NOTIFIED_SEND_REACTION" || type === "SEND_REACTION" ||
          type === "140" || type === "139"
        ) {
          runtime.invalidateMessages("");
          emitEvent("reaction_changed");
        } else if (type === "NOTIFIED_READ_MESSAGE") {
          emitEvent("read_receipt", {
            chatMid: String(op.param1 ?? ""),
            userMid: String(op.param2 ?? ""),
            messageId: String(op.param3 ?? ""),
          });
        } else if (
          type === "ADD_CONTACT" ||
          type === "4" ||
          type === "FRIEND_REQUEST_ACCEPTED" ||
          type === "63"
        ) {
          const mid = String(op.param1 ?? op.param2 ?? "");
          if (mid) {
            void Promise.resolve(runtime.onContactAdded?.(mid)).catch(
              (error) => console.error("[listen contact_added]", mid, error),
            );
          }
        } else if (
          type === "NOTIFIED_UPDATE_PROFILE" ||
          type === "2" ||
          type === "NOTIFIED_BUDDY_UPDATE_PROFILE" ||
          type === "78"
        ) {
          const mid = String(op.param1 ?? "");
          if (mid) {
            void Promise.resolve(runtime.onProfileUpdated?.(mid)).catch(
              (error) => console.error("[listen profile_updated]", mid, error),
            );
          }
        }
      } catch (e) {
        console.error("[listen event]", e);
      }
    };
    const onIncoming: typeof calls.handleIncomingCall = (event) => {
      if (ownsSession()) return calls.handleIncomingCall(event);
    };
    const onCancel: typeof calls.handleCallCancel = (event) => {
      if (ownsSession()) return calls.handleCallCancel(event);
    };
    const onLog = ({ type, data }: { type: string; data: unknown }) => {
      // Pinned LINE.js emits this for actual PUSH bytes, including idle pings.
      if (
        ownsSession() && type === "[LEGY/PUSH] readByte" &&
        data instanceof Uint8Array && data.byteLength > 0
      ) lastProgressAt = clock.now();
    };
    const cancelTimer = clock.every(() => {
      if (!ownsSession()) {
        if (active === session) stop();
        return;
      }
      if (clock.now() - lastProgressAt < 120_000) return;
      stop();
      runtime.onTransportStalled();
    }, 10_000);
    session.dispose = () => {
      cancelTimer();
      sentMessageIds.clear();
      client.off("message", onMessage);
      client.off("event", onEvent);
      client.off("call:incoming", onIncoming);
      client.off("call:cancel", onCancel);
      client.base.off("log", onLog);
      controller.abort();
    };
    client.on("message", onMessage);
    client.on("event", onEvent);
    client.on("call:incoming", onIncoming);
    client.on("call:cancel", onCancel);
    client.base.on("log", onLog);
    try {
      client.listen({ talk: true, square: false, signal: controller.signal });
    } catch (error) {
      stop();
      throw error;
    }
    emitEvent("listening");
  }

  return { start: startListen, stop };
}
