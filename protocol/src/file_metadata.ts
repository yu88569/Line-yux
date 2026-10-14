export type FileMetadata = {
  fileName: string | null;
  fileSize: number | null;
};

export const FILE_METADATA_VERSION = 1;

export function attachmentName(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const name = value.trim();
  if (!name || /^\[(?:file|attachment)\]$/i.test(name)) return null;
  if (
    Array.from(name).some((char) =>
      char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127
    )
  ) return null;
  return name;
}

export function attachmentSize(value: unknown): number | null {
  if (typeof value !== "number" && typeof value !== "string") return null;
  if (typeof value === "string" && !/^\d+$/.test(value)) return null;
  const size = Number(value);
  return Number.isSafeInteger(size) && size >= 0 ? size : null;
}

export async function resolveFileMetadata({
  metadata,
  text,
  chunks,
  decryptEnvelope,
}: {
  metadata: Record<string, unknown>;
  text: unknown;
  chunks?: readonly unknown[] | null;
  decryptEnvelope: () => Promise<unknown>;
}): Promise<FileMetadata> {
  const encrypted = Boolean(metadata.e2eeVersion) || (chunks?.length ?? 0) >= 5;
  let fileName = attachmentName(metadata.FILE_NAME) ||
    attachmentName(metadata.FILENAME);
  if (!fileName && (chunks?.length ?? 0) >= 5) {
    try {
      const envelope = await decryptEnvelope();
      if (envelope && typeof envelope === "object" && "fileName" in envelope) {
        fileName = attachmentName(envelope.fileName);
      }
    } catch { /* Metadata may be unavailable for expired encryption keys. */ }
  }
  return {
    fileName: fileName || attachmentName(text),
    fileSize: encrypted ? null : attachmentSize(metadata.FILE_SIZE),
  };
}

export function needsFileMetadataRefresh(
  messages: Record<string, unknown>[],
): boolean {
  return messages.some((message) =>
    message.contentType === "FILE" &&
    message.fileMetadataVersion !== FILE_METADATA_VERSION
  );
}
