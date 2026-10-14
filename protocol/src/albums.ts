import type { Client } from "@evex/linejs";

export const ALBUM_METADATA_VERSION = 1;
export type AlbumRef = { chatMid: string; albumId: string; title: string };
export type AlbumPhoto = { oid: string; kind: "image" | "video" };
export type AlbumPage = { photos: AlbumPhoto[]; nextCursor: string | null };

function field(value: unknown, key: string): unknown {
  return value !== null && typeof value === "object" && key in value
    ? Reflect.get(value, key)
    : undefined;
}
function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}
function pathId(value: unknown): string {
  const id = text(value);
  if (
    !id || id === "." || id === ".." ||
    Array.from(id).some((c) => c.charCodeAt(0) < 32)
  ) {
    throw new Error("Invalid album resource ID");
  }
  return id;
}
export function extractAlbum(metadata: unknown): AlbumRef | null {
  if (field(metadata, "serviceType") !== "AB") return null;
  let url: URL | null = null;
  try {
    const raw = text(field(metadata, "postEndUrl"));
    if (raw) url = new URL(raw);
  } catch { /* Metadata may omit the deep link. */ }
  const albumId = text(field(metadata, "cafeId")) ||
    url?.searchParams.get("albumIdV2") || url?.searchParams.get("albumId");
  const chatMid = text(field(metadata, "chatId")) ||
    url?.searchParams.get("homeId");
  if (
    !albumId || !/^\d+$/.test(albumId) || albumId === "0" || !chatMid ||
    !/^[cou][a-f0-9]{32}$/.test(chatMid)
  ) return null;
  return {
    albumId,
    chatMid,
    title: text(field(metadata, "albumName")) || "Album",
  };
}
export function needsAlbumMetadataRefresh(
  messages: Record<string, unknown>[],
): boolean {
  return messages.some((m) =>
    m.contentType === "POSTNOTIFICATION" &&
    m.albumMetadataVersion !== ALBUM_METADATA_VERSION
  );
}
export function parseAlbumPage(value: unknown): AlbumPage {
  const code = field(value, "code");
  if (code !== undefined && code !== 0) {
    throw new Error(
      `Album service error ${code}: ${field(value, "message") ?? ""}`,
    );
  }
  const result = field(value, "result");
  const rows = field(result, "photos");
  if (!Array.isArray(rows)) throw new Error("Invalid album photo response");
  const photos = rows.map((row): AlbumPhoto => {
    const resource = field(row, "obsResourceId");
    const oid = pathId(field(resource, "oid") ?? field(row, "oid"));
    const sid = field(resource, "sid");
    const resourceType = field(row, "resourceType");
    if (sid !== undefined && sid !== "a" && sid !== "v") {
      throw new Error("Unsupported album resource type");
    }
    return {
      oid,
      kind: sid === "v" || resourceType === "VIDEO" ? "video" : "image",
    };
  });
  const cursor = field(result, "nextCursor");
  if (cursor !== undefined && cursor !== null && typeof cursor !== "string") {
    throw new Error("Invalid album pagination cursor");
  }
  return { photos, nextCursor: text(cursor) };
}

// Transport mirrors the read-only Moa service in upstream linejs.
type AlbumBase =
  & Pick<
    Client["base"],
    "fetch" | "endpoint" | "request" | "channel" | "config"
  >
  & { profile?: { mid: string } };

export class AlbumService {
  constructor(private base: AlbumBase) {}
  private token: Promise<string> | null = null;
  private async channelToken(): Promise<string> {
    if (!this.token) {
      this.token = this.base.channel.approveChannelAndIssueChannelToken({
        channelId: "1375220249",
      }).then((r) => {
        const token = text(r?.channelAccessToken);
        if (!token) throw new Error("Album channel token missing");
        return token;
      });
    }
    try {
      return await this.token;
    } catch (e) {
      this.token = null;
      throw e;
    }
  }
  private async fetch(
    path: string,
    headers: Record<string, string>,
  ): Promise<Response> {
    const token = await this.channelToken();
    const response = await this.base.fetch(
      `https://${this.base.endpoint}${path}`,
      {
        method: "POST",
        headers: {
          ...this.base.request.getHeader("GET"),
          "X-Line-ChannelToken": token,
          ...headers,
        },
        body: new Uint8Array(),
        signal: AbortSignal.timeout(this.base.config.timeout),
      },
    );
    if (!response.ok) {
      this.token = null;
      throw new Error(`Album HTTP ${response.status}`);
    }
    return response;
  }
  async photos(album: AlbumRef, cursor = ""): Promise<AlbumPage> {
    const params = new URLSearchParams({
      cursor,
      pageSize: "24",
      orderBy: "createTimeDesc",
      include: "all",
      filterType: "",
    });
    const response = await this.fetch(
      `/ext/album/api/v6/albums/${
        encodeURIComponent(pathId(album.albumId))
      }/photos?${params}`,
      {
        accept: "application/json",
        "content-type": "application/json; charset=UTF-8",
        "X-Line-Mid": this.base.profile?.mid ?? "",
        "X-Line-Chat-Id": album.chatMid,
      },
    );
    try {
      return parseAlbumPage(await response.json());
    } catch (error) {
      this.token = null;
      throw error;
    }
  }
  async download(album: AlbumRef, photo: AlbumPhoto): Promise<Uint8Array> {
    const response = await this.fetch(
      `/oa/r/album/${photo.kind === "video" ? "v" : "a"}/${
        encodeURIComponent(pathId(photo.oid))
      }`,
      {
        "X-Line-Album": pathId(album.albumId),
        "X-Line-Mid": album.chatMid,
      },
    );
    return new Uint8Array(await response.arrayBuffer());
  }
}

const services = new WeakMap<Client["base"], AlbumService>();
export function albumService(base: Client["base"]): AlbumService {
  const existing = services.get(base);
  if (existing) return existing;
  const service = new AlbumService(base);
  services.set(base, service);
  return service;
}
