import { assertEquals } from "@std/assert";
import { Client } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { join } from "@std/path";
import {
  createOutgoingMedia,
  type OutgoingMediaRuntime,
} from "../src/outgoing_media.ts";

Deno.test("small FILE downloads retain original name and reuse the exact local payload", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const data = new TextEncoder().encode("OK");
    const client = new Client(new BaseClient({ device: "ANDROIDSECONDARY" }));
    let fetches = 0;
    let result: unknown;
    const card: Record<string, unknown> = {
      id: "100",
      contentType: "FILE",
      fileName: "[File]",
    };
    const pathFor = (id: string, ext = "jpg") =>
      join(directory, `${id}.${ext}`);
    const runtime: OutgoingMediaRuntime = {
      getClient: () => client,
      mediaDir: directory,
      mediaDest: pathFor,
      fullDest: pathFor,
      thumbDest: pathFor,
      existingFile: () => Promise.resolve(null),
      existingImage: () => Promise.resolve(null),
      isImageBytes: () => false,
      writeImageFile: () => Promise.resolve(null),
      writeFullImage: () => Promise.resolve(null),
      refetchMessageData: () => {
        fetches++;
        return Promise.resolve({
          buf: data,
          mime: "text/plain",
          fileName: "note.txt",
        });
      },
      uiMediaPath: () => Promise.resolve(null),
      isPreviewOrThumbImage: () => Promise.resolve(false),
      getBoxCursor: () => undefined,
      isMineFrom: () => false,
      normType: (value) => String(value),
      getCachedMessages: () => [card],
      updateDownloadedFile: (_chat, _id, file) => {
        Object.assign(card, file);
        return Promise.resolve();
      },
      invalidateMessages: () => {},
      invalidateChats: () => {},
      sentMessagePayload: () => ({}),
      touchChatPreviewFromMessage: () => {},
      emitEvent: () => {},
      ok: (_id, payload) => {
        result = payload;
      },
      fail: (_id, error) => {
        throw new Error(error);
      },
    };
    const media = createOutgoingMedia(runtime);
    await media.downloadMedia(1, "chat", "100");
    const path = join(directory, "100.note.txt");
    assertEquals(result, {
      path,
      fileName: "note.txt",
      contentType: "FILE",
      bytes: 2,
      cached: false,
    });
    assertEquals(await Deno.readFile(path), data);
    assertEquals(card.fileSize, 2);
    await media.downloadMedia(2, "chat", "100");
    assertEquals(result, {
      path,
      fileName: "note.txt",
      contentType: "FILE",
      bytes: 2,
      cached: true,
    });
    assertEquals(fetches, 1);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
