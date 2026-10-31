import { assertEquals } from "@std/assert";
import {
  createMessageView,
  FLEX_METADATA_VERSION,
  needsFlexMetadataRefresh,
} from "../src/message_view.ts";

const { extractFlex } = createMessageView({
  normType: (value) => String(value),
  existingFullImage: () => Promise.resolve(null),
  existingImage: () => Promise.resolve(null),
  mediaDest: (id) => id,
  thumbDest: (id) => id,
  isValidAudioFile: () => Promise.resolve(false),
  uiMediaPath: () => Promise.resolve(null),
  stickerDir: "",
});

Deno.test("Flex preserves nested layout and every appointment field", () => {
  const contents = {
    type: "bubble",
    header: {
      type: "box",
      layout: "vertical",
      backgroundColor: "#0c6b5c",
      contents: [{ type: "text", text: "แจ้งเตือนล่วงหน้า 1 วัน" }],
    },
    body: {
      type: "box",
      layout: "vertical",
      contents: Array.from(
        { length: 30 },
        (_, i) => ({ type: "text", text: `Appointment field ${i}` }),
      ),
    },
  };
  const result = extractFlex({ FLEX_JSON: JSON.stringify(contents) });
  assertEquals(result?.contents, contents);
  assertEquals(Array.isArray(result?.texts) && result.texts.length, 31);
  assertEquals(
    extractFlex({
      FLEX_JSON: JSON.stringify({
        type: "flex",
        contents,
        altText: "Schedule",
      }),
    })?.contents,
    contents,
  );
});

Deno.test("Flex message action sends text instead of the button label", () => {
  const result = extractFlex({
    FLEX_JSON: JSON.stringify({
      type: "bubble",
      body: {
        type: "box",
        contents: [{
          type: "button",
          action: {
            type: "message",
            label: "Details",
            text: "Show appointment",
          },
        }],
      },
    }),
  });
  assertEquals(result?.actions, [{
    kind: "message",
    label: "Details",
    data: "Show appointment",
    uri: null,
  }]);
});

Deno.test("Malformed Flex keeps alt text and old cached messages refresh once", () => {
  assertEquals(extractFlex({ FLEX_JSON: "{", ALT_TEXT: "Schedule" }), {
    altText: "Schedule",
    texts: [],
    actions: [],
    contents: null,
  });
  assertEquals(extractFlex({ FLEX_JSON: "{" }), null);
  assertEquals(
    needsFlexMetadataRefresh([{ contentType: "FLEX", flex: {} }]),
    true,
  );
  assertEquals(
    needsFlexMetadataRefresh([{
      contentType: "FLEX",
      flexMetadataVersion: FLEX_METADATA_VERSION,
    }]),
    false,
  );
  assertEquals(needsFlexMetadataRefresh([{ contentType: "NONE" }]), false);
});
