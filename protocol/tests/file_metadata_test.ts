import { assertEquals } from "@std/assert";
import {
  attachmentName,
  attachmentSize,
  FILE_METADATA_VERSION,
  needsFileMetadataRefresh,
  resolveFileMetadata,
} from "../src/file_metadata.ts";

Deno.test("plaintext file metadata preserves real name and safe byte size without decryption", async () => {
  const result = await resolveFileMetadata({
    metadata: { FILE_NAME: "รายงาน.xlsx", FILE_SIZE: "1234" },
    text: "[File]",
    decryptEnvelope: () => {
      throw new Error("unexpected decrypt");
    },
  });
  assertEquals(result, { fileName: "รายงาน.xlsx", fileSize: 1234 });
});

Deno.test("E2EE file card decrypts only its envelope and does not report ciphertext length", async () => {
  let calls = 0;
  const result = await resolveFileMetadata({
    metadata: { e2eeVersion: "2", FILE_SIZE: "1280" },
    text: "[File]",
    chunks: ["a", "b", "c", "d", "e"],
    decryptEnvelope: () => {
      calls++;
      return Promise.resolve({
        fileName: "presentation.pptx",
        keyMaterial: "secret",
      });
    },
  });
  assertEquals(result, { fileName: "presentation.pptx", fileSize: null });
  assertEquals(calls, 1);
});

Deno.test("unavailable envelope keeps unknown filename null and valid text remains a fallback", async () => {
  const input = {
    metadata: {},
    chunks: [1, 2, 3, 4, 5],
    decryptEnvelope: () => Promise.reject(new Error("expired key")),
  };
  assertEquals(await resolveFileMetadata({ ...input, text: "[File]" }), {
    fileName: null,
    fileSize: null,
  });
  assertEquals(
    (await resolveFileMetadata({ ...input, text: "document.pdf" })).fileName,
    "document.pdf",
  );
});

Deno.test("external metadata rejects placeholders, control characters and unsafe sizes", () => {
  for (const value of ["[File]", " [Attachment] ", "", "bad\nname.txt", null]) {
    assertEquals(attachmentName(value), null);
  }
  for (
    const value of [
      -1,
      "-1",
      "",
      "NaN",
      "1.2",
      "1e3",
      Number.MAX_SAFE_INTEGER + 1,
      null,
    ]
  ) {
    assertEquals(attachmentSize(value), null);
  }
  assertEquals(attachmentSize("0"), 0);
});

Deno.test("legacy file cards refresh once without repeatedly fetching unresolved files", () => {
  assertEquals(
    needsFileMetadataRefresh([{ contentType: "FILE", fileName: "[File]" }]),
    true,
  );
  assertEquals(
    needsFileMetadataRefresh([{
      contentType: "FILE",
      fileName: null,
      fileMetadataVersion: FILE_METADATA_VERSION,
    }]),
    false,
  );
});
