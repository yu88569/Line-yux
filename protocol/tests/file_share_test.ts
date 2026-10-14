import { assertEquals } from "@std/assert";
import { Client } from "@evex/linejs";
import { BaseClient } from "@evex/linejs/base";
import { join } from "@std/path";
import {
  createOutgoingMedia,
  type OutgoingMediaRuntime,
} from "../src/outgoing_media.ts";

Deno.test("sharing tiny FILEs keeps the target, original name and FILE type at the upload boundary", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const client = new Client(
      new BaseClient({
        device: "ANDROIDSECONDARY",
        fetch: () => {
          throw new Error("Network forbidden in file sharing test");
        },
      }),
    );
    const uploads: {
      chat: string;
      type: string;
      name: string;
      bytes: number[];
    }[] = [];
    Object.defineProperty(client.base.obs, "uploadObjTalk", {
      value: async (
        chat: string,
        type: string,
        blob: Blob,
        _unused: unknown,
        name: string,
      ) => {
        uploads.push({
          chat,
          type,
          name,
          bytes: Array.from(new Uint8Array(await blob.arrayBuffer())),
        });
        return { objId: "sent-file" };
      },
    });
    Object.defineProperty(
      client.base.talk,
      "getPreviousMessagesV2WithRequest",
      { value: () => Promise.resolve([]) },
    );
    const pathFor = (id: string, ext = "jpg") =>
      join(directory, `${id}.${ext}`);
    let result: unknown;
    let failure: string | null = null;
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
      refetchMessageData: () => Promise.resolve(null),
      uiMediaPath: () => Promise.resolve(null),
      isPreviewOrThumbImage: () => Promise.resolve(false),
      getBoxCursor: () => undefined,
      isMineFrom: () => false,
      normType: (value) => String(value),
      getCachedMessages: () => undefined,
      updateDownloadedFile: () => Promise.resolve(),
      invalidateMessages: () => {},
      invalidateChats: () => {},
      sentMessagePayload: (_sent, chatMid, text) => ({ chatMid, text }),
      touchChatPreviewFromMessage: () => {},
      emitEvent: () => {},
      ok: (_id, payload) => {
        result = payload;
      },
      fail: (_id, error) => {
        failure = error;
      },
    };
    const source = join(directory, "123-cached.bin");
    const bytes = new TextEncoder().encode("OK");
    await Deno.writeFile(source, bytes);
    const media = createOutgoingMedia(runtime);
    await media.sendMedia(
      1,
      "r-target",
      source,
      "file",
      undefined,
      "รายงาน.png",
    );
    assertEquals(failure, null);
    assertEquals(uploads, [{
      chat: "r-target",
      type: "file",
      name: "รายงาน.png",
      bytes: [79, 75],
    }]);
    assertEquals(
      result && typeof result === "object" && "message" in result
        ? result.message
        : null,
      {
        chatMid: "r-target",
        text: "รายงาน.png",
        id: "sent-file",
        contentType: "FILE",
        imagePath: null,
        audioPath: null,
        fileName: "รายงาน.png",
        fileSize: 2,
        filePath: join(directory, "sent-file.123-cached.bin"),
        durationMs: null,
        needsMedia: false,
      },
    );
    await media.sendMedia(2, "r-target", source, "image");
    assertEquals(failure, "file_too_small");
    assertEquals(uploads.length, 1);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
