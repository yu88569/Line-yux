import { assertEquals, assertRejects, assertThrows } from "@std/assert";
import { BaseClient } from "@evex/linejs/base";
import { Protocols, Thrift } from "@evex/linejs/thrift";
import {
  AlbumService,
  extractAlbum,
  needsAlbumMetadataRefresh,
  parseAlbumPage,
} from "../src/albums.ts";

const chatMid = `c${"1".repeat(32)}`;
const album = { chatMid, albumId: "5333564955743123169", title: "Test album" };
Deno.test("album metadata keeps large IDs and album home identity for DM notifications", () => {
  assertEquals(
    extractAlbum({
      serviceType: "AB",
      albumName: album.title,
      cafeId: album.albumId,
      chatId: chatMid,
    }),
    album,
  );
  assertEquals(
    extractAlbum({
      serviceType: "AB",
      albumName: album.title,
      postEndUrl:
        `line://group/home/albums/album?albumId=${album.albumId}&homeId=${chatMid}`,
    }),
    album,
  );
  assertEquals(
    extractAlbum({ serviceType: "GB", cafeId: album.albumId, chatId: chatMid }),
    null,
  );
  assertEquals(
    extractAlbum({ serviceType: "AB", cafeId: "0", chatId: chatMid }),
    null,
  );
  assertEquals(
    needsAlbumMetadataRefresh([{ contentType: "POSTNOTIFICATION" }]),
    true,
  );
  assertEquals(
    needsAlbumMetadataRefresh([{
      contentType: "POSTNOTIFICATION",
      albumMetadataVersion: 1,
      album: null,
    }]),
    false,
  );
});
Deno.test("album photo response parses images and videos and rejects unsafe resources", () => {
  assertEquals(
    parseAlbumPage({
      code: 0,
      result: {
        photos: [{ obsResourceId: { oid: "photo.1", sid: "a" } }, {
          oid: "video.1",
          resourceType: "VIDEO",
        }],
        nextCursor: "page2",
      },
    }),
    {
      photos: [{ oid: "photo.1", kind: "image" }, {
        oid: "video.1",
        kind: "video",
      }],
      nextCursor: "page2",
    },
  );
  assertThrows(() => parseAlbumPage({ code: 1 }), Error);
  assertThrows(
    () => parseAlbumPage({ result: { photos: [{ oid: ".." }] } }),
    Error,
  );
});
Deno.test("compatible Moa transport uses album token and exact JSON and OBS headers", async () => {
  const requests: Request[] = [];
  const thrift = new Thrift();
  const base = new BaseClient({
    device: "ANDROIDSECONDARY",
    fetch: async (input) => {
      const request = new Request(input);
      await request.clone().arrayBuffer();
      requests.push(request);
      const url = new URL(request.url);
      if (
        !url.pathname.startsWith("/ext/album") &&
        !url.pathname.startsWith("/oa/r")
      ) {
        const bytes = thrift.writeThrift(
          [[12, 0, [[11, 5, "test-channel-token"]]]],
          "approveChannelAndIssueChannelToken",
          Protocols[4],
        );
        bytes[1] = 0x41;
        return new Response(new Uint8Array(bytes).buffer);
      }
      if (url.pathname.startsWith("/oa/r")) {
        return new Response(new Uint8Array([1, 2, 3]));
      }
      return Response.json({
        code: 0,
        result: {
          photos: [{ obsResourceId: { oid: "photo.1", sid: "a" } }],
          nextCursor: "page2",
        },
      });
    },
  });
  base.authToken = "test-auth";
  const service = new AlbumService({
    fetch: base.fetch,
    endpoint: base.endpoint,
    request: base.request,
    channel: base.channel,
    config: base.config,
    profile: { mid: `u${"2".repeat(32)}` },
  });
  assertEquals((await service.photos(album, "page1")).nextCursor, "page2");
  assertEquals(
    await service.download(album, { oid: "photo.1", kind: "image" }),
    new Uint8Array([1, 2, 3]),
  );
  assertEquals(requests.length, 3);
  const list = requests[1];
  assertEquals(list.method, "POST");
  assertEquals(list.headers.get("accept"), "application/json");
  assertEquals(list.headers.get("X-Line-Chat-Id"), chatMid);
  assertEquals(list.headers.get("X-Line-ChannelToken"), "test-channel-token");
  assertEquals(new URL(list.url).searchParams.get("cursor"), "page1");
  assertEquals(requests[2].headers.get("X-Line-Album"), album.albumId);
  assertEquals(requests[2].headers.get("X-Line-Mid"), chatMid);
  assertEquals(new URL(requests[2].url).pathname, "/oa/r/album/a/photo.1");
});
Deno.test("album HTTP failure reaches the caller", async () => {
  const base = new BaseClient({
    device: "ANDROIDSECONDARY",
    fetch: () => Promise.resolve(new Response("", { status: 503 })),
  });
  await assertRejects(() => new AlbumService(base).photos(album));
});
