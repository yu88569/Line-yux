import { parseLocation } from "./locations.ts";
import { isMediaType } from "./messages.ts";
import { attachmentName } from "./file_metadata.ts";
import { join } from "@std/path";

type Json = Record<string, unknown>;

export const FLEX_METADATA_VERSION = 1;

export function needsFlexMetadataRefresh(messages: Json[]): boolean {
  return messages.some((message) =>
    message.contentType === "FLEX" &&
    message.flexMetadataVersion !== FLEX_METADATA_VERSION
  );
}

export type MessageViewRuntime = {
  normType: (value: unknown) => string;
  existingFullImage: (id: string) => Promise<string | null>;
  existingImage: (path: string) => Promise<string | null>;
  mediaDest: (id: string, ext?: string) => string;
  thumbDest: (id: string) => string;
  isValidAudioFile: (path: string | null) => Promise<boolean>;
  uiMediaPath: (id: string, path: string) => Promise<string | null>;
  stickerDir: string;
};

export function createMessageView(runtime: MessageViewRuntime) {
  const {
    normType,
    existingFullImage,
    existingImage,
    mediaDest,
    thumbDest,
    isValidAudioFile,
    uiMediaPath,
    stickerDir,
  } = runtime;

  function previewBody(msg: Json): string {
    const ct = normType(msg.contentType);
    const text = String(msg.text ?? "").trim();
    const L = previewLang();
    if (ct === "LOCATION") {
      const location = parseLocation(msg.location);
      return location?.title ? `${L.location}: ${location.title}` : L.location;
    }
    if (ct === "IMAGE") return L.photo;
    if (ct === "VIDEO") return L.video;
    if (ct === "STICKER") return L.sticker;
    if (ct === "AUDIO") return L.voice;
    if (ct === "FILE") {
      const name = attachmentName(msg.fileName);
      return name ? `${L.file}: ${name}` : L.file;
    }
    if (ct === "FLEX") return text || L.flex;
    if (!text) return ct !== "NONE" ? ct.toLowerCase() : L.message;
    return text.length > 64 ? `${text.slice(0, 63)}…` : text;
  }

  function previewLang() {
    const code = (Deno.env.get("LINE_GTK_LANG") || "th").toLowerCase();
    const th = code === "th" || code === "thai";
    return th
      ? {
        you: "คุณ",
        they: "เขา",
        photo: "รูปภาพ",
        video: "วิดีโอ",
        sticker: "สติกเกอร์",
        voice: "ข้อความเสียง",
        file: "ไฟล์",
        location: "พิกัด",
        flex: "ข้อความ Flex",
        message: "ข้อความ",
        tap: "แตะเพื่อเปิด",
      }
      : {
        you: "You",
        they: "They",
        photo: "Photo",
        video: "Video",
        sticker: "Sticker",
        voice: "Voice message",
        file: "File",
        location: "Location",
        flex: "Flex message",
        message: "Message",
        tap: "Tap to open",
      };
  }

  function previewLine(msg: Json): string {
    const L = previewLang();
    const who = msg.mine ? L.you : L.they;
    return `${who}: ${previewBody(msg)}`;
  }

  function extractFlex(meta: Record<string, string>): Json | null {
    let altText = meta.ALT_TEXT || "";
    let root: unknown = null;
    if (meta.FLEX_JSON) {
      try {
        root = JSON.parse(meta.FLEX_JSON);
      } catch {
        root = null;
      }
    }
    if (
      root && typeof root === "object" && "type" in root && root.type === "flex"
    ) {
      if (!altText && "altText" in root && typeof root.altText === "string") {
        altText = root.altText;
      }
      root = "contents" in root ? root.contents : null;
    }
    const hasContents = root && typeof root === "object" && "type" in root &&
      ["bubble", "carousel", "box"].includes(String(root.type));
    const texts: string[] = [];
    const actions: Json[] = [];
    const seen = new Set<string>();

    const walk = (node: unknown, depth = 0) => {
      if (!node || depth > 12) return;
      if (Array.isArray(node)) {
        for (const n of node) walk(n, depth + 1);
        return;
      }
      if (typeof node !== "object") return;
      const o = node as Record<string, unknown>;
      if (typeof o.text === "string" && o.text.trim()) {
        const t = o.text.trim();
        if (!texts.includes(t)) texts.push(t);
      }
      if (o.action && typeof o.action === "object") {
        const a = o.action as Record<string, unknown>;
        const label = String(a.label || o.text || a.data || a.uri || "Action");
        const kind = String(a.type || "postback").toLowerCase();
        const data = a.text != null && kind === "message"
          ? String(a.text)
          : a.data != null
          ? String(a.data)
          : null;
        const uri = a.uri != null
          ? String(a.uri)
          : (a.url != null ? String(a.url) : null);
        const key = `${kind}|${label}|${data || ""}|${uri || ""}`;
        if (!seen.has(key) && actions.length < 24) {
          seen.add(key);
          actions.push({ label, kind, data, uri });
        }
      }
      for (const v of Object.values(o)) walk(v, depth + 1);
    };
    walk(root);

    if (!altText && !texts.length && !actions.length && !hasContents) {
      return null;
    }
    return {
      altText: altText || texts[0] || "Flex message",
      texts,
      actions,
      contents: root,
    };
  }

  /** Serve disk-cached stickers/thumbs immediately; only hydrate misses. */
  async function forUiMessages(messages: Json[]): Promise<Json[]> {
    return await Promise.all(messages.map(async (m) => {
      const ct = normType(m.contentType);
      const id = String(m.id ?? "");
      let imagePath: string | null = (m.imagePath as string) || null;
      let audioPath: string | null = (m.audioPath as string) || null;
      let needsMedia = isMediaType(ct) && !imagePath && !audioPath;

      if (ct === "STICKER") {
        const stkId = String(m.stickerId || "");
        if (stkId) {
          const anim = await existingImage(
            join(stickerDir, `${stkId}.anim.png`),
          );
          const st = await existingImage(join(stickerDir, `${stkId}.png`));
          if (anim) {
            imagePath = anim;
            needsMedia = false;
          } else if (st) {
            // Show static now; hydrate may upgrade to APNG.
            imagePath = st;
            needsMedia = true;
          }
        }
      } else if (ct === "IMAGE" || ct === "VIDEO") {
        const cached = await existingImage(thumbDest(id)) ||
          await existingFullImage(id) ||
          await existingImage(mediaDest(id)) ||
          await existingImage(mediaDest(id, "png"));
        if (cached) {
          imagePath = (await uiMediaPath(id, cached)) || cached;
          needsMedia = false;
        }
      } else if (ct === "AUDIO") {
        audioPath = null;
        for (const ext of ["m4a", "mp3", "aac", "ogg"]) {
          const p = mediaDest(id, ext);
          if (await isValidAudioFile(p)) {
            audioPath = p;
            break;
          }
        }
        if (audioPath) needsMedia = false;
        else needsMedia = true;
      } else if (ct === "FILE") {
        // Filename-only bubble; download is on demand / background, not a preview.
        needsMedia = false;
      }

      return { ...m, imagePath, audioPath, needsMedia };
    }));
  }

  return {
    previewBody,
    previewLang,
    previewLine,
    extractFlex,
    forUiMessages,
  };
}
