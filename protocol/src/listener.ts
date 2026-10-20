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
  upsertChatFromContact: (mid: string) => Promise<void>;
  emitEvent: (event: string, payload?: Json) => void;
};

export function createListener(runtime: ListenerRuntime) {
  const {
    summarizeTalkMessage,
    storeBoxCursor,
    upsertChatFromMessage,
    hydrateLiveMedia,
    upsertChatFromContact,
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
    runtime.setListening(true);

    // Register BEFORE listen() so early ops are not missed.
    const onMessage = async (msg: TalkMessage) => {
      if (!ownsSession()) return;
      try {
        const message = await summarizeTalkMessage(msg, { withMedia: false });
        if (!ownsSession()) return;
        const peer = messageChatMid(message);
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
    }) => {
      if (!ownsSession()) return;
      try {
        const type = String(op?.type ?? "");
        if (type === "NOTIFIED_READ_MESSAGE") {
          emitEvent("read_receipt", {
            chatMid: String(op.param1 ?? ""),
            userMid: String(op.param2 ?? ""),
            messageId: String(op.param3 ?? ""),
          });
        } else if (
          type.includes("ADD_CONTACT") ||
          type.includes("ACCEPT_CONTACT") ||
          type.includes("FRIEND_REQUEST") ||
          type === "NOTIFIED_UPDATE_PROFILE"
        ) {
          const mid = String(op.param1 ?? op.param2 ?? "");
          if (mid) void upsertChatFromContact(mid);
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
