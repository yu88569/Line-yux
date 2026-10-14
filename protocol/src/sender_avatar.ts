type SenderAvatarRuntime = {
  existingPath: (mid: string) => Promise<string | null>;
  resolveContact: (mid: string) => Promise<{ picturePath: string | null }>;
  avatarPathFor: (mid: string, picturePath: string) => Promise<string | null>;
  dedupe: <T>(key: string, fn: () => Promise<T>) => Promise<T>;
  emitReady: (mid: string, path: string) => void;
};

export async function resolveSenderAvatar(
  sender: unknown,
  runtime: SenderAvatarRuntime,
): Promise<string | null> {
  if (typeof sender !== "string" || !/^u[0-9a-f]{32}$/i.test(sender)) {
    return null;
  }
  return await runtime.dedupe(`sender-avatar:${sender}`, async () => {
    try {
      let path = await runtime.existingPath(sender);
      if (!path) {
        const contact = await runtime.resolveContact(sender);
        if (contact.picturePath) {
          path = await runtime.avatarPathFor(sender, contact.picturePath);
        }
      }
      if (path) runtime.emitReady(sender, path);
      return path;
    } catch {
      return null;
    }
  });
}
