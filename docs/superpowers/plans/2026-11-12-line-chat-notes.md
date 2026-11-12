# LINE Chat Notes Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans when the user requests implementation. Checkboxes track work; this document does not authorize execution.

**Goal:** Add native LINE group-chat notes with reading, text creation, editing, deletion, and comments.

**Architecture:** A TypeScript sidecar adapter validates LINE Timeline responses and exposes chat-scoped commands. A GTK notes window consumes typed responses through existing sidecar and pending-request patterns.

**Tech Stack:** Rust, GTK4/libadwaita, Deno, pinned `@evex/linejs@3.2.1`.

**Spec:** `docs/superpowers/specs/2026-11-12-line-chat-notes-design.md`.

## Global Constraints

- Preserve existing uncommitted attachment changes in `src/ui/media.rs` and `src/ui/file_action_tests.rs`.
- Use LINE notes and the existing session; never replace them with local-only notes.
- Initially support verified group chats; explain unsupported 1:1 and legacy rooms.
- New posts and comments are text-only. Preserve existing attachments or disable editing rich notes.
- Editing and deletion require verified ownership; unknown ownership denies both.
- Never automatically retry a write after timeout. Never create or delete real posts during automated tests.
- Use `TALKROOM` and verified group permissions; do not rely on `TIMELINE` or `ALL` defaults.
- Do not invent response schemas or permissions. If discovery fails, keep the affected operation unavailable and report the evidence.

## Review Focus

1. A late response after a chat switch must remain bound to its original chat (task 4).
2. A post with image or sticker must not lose attachments during edit (tasks 2 and 4).
3. An ambiguous timeout must preserve the draft without sending it again (tasks 3 and 4).
4. Unknown author and server rejection must never be shown as success (tasks 2–4).
5. Missing, repeated, or invalid pagination cursors must not create endless requests or hide errors (tasks 2 and 4).

## File Map

- Create `protocol/src/notes.ts`: validated adapter and normalized note/comment models.
- Create `protocol/tests/notes_test.ts`: sanitized fixtures and fake-transport behavior tests.
- Modify `protocol/src/main.ts`: dispatch note commands using the active client.
- Modify `src/protocol.rs`: deserialize normalized note/comment responses.
- Create `src/ui/notes.rs`: GTK window, state and action handlers.
- Modify `src/ui/shell.rs`, `src/ui/mod.rs`, `src/ui/state.rs`, `src/ui/events.rs`: entry point and pending responses.
- Modify `assets/lang/thai.json` and `assets/lang/eng.json`: visible strings.
- Add GTK fixture in `src/ui/notes.rs` or a dedicated `src/ui/notes_tests.rs` following the album fixture.

### Task 1: Establish native protocol evidence

**Files:** `protocol/src/notes.ts`, `protocol/tests/notes_test.ts` (later tasks); capture sanitized evidence in `.audit/notes-protocol/`.

- [ ] Inspect pinned SDK Timeline implementation and primary sources for group home ID, chat-scoped write permissions, response fields, ownership, note pagination, comment list endpoint and pagination. Compare against the selected session's device type without printing credentials or message contents.
- [ ] If possible perform a read-only request against a user-accessible group and save only sanitized structural fixtures. Do not publish a note or comment. Record endpoint, request shape, response envelope, pagination and exact evidence source.
- [ ] Gate each operation on independently verified evidence. If comment listing, permissions, or ownership cannot be established, record the blocker rather than implementing a speculative endpoint.

### Task 2: Validated notes adapter

**Files:** Create `protocol/src/notes.ts` and `protocol/tests/notes_test.ts`.

**Interfaces:** Export `Note`, `NotePage`, `NoteComment`, `CommentPage` with stable IDs, author ID, text, creation time, attachment indicators, and opaque verified pagination cursor. Export `NotesService` methods `list(chatMid, cursor)`, `get(chatMid, noteId)`, `create(chatMid, text)`, `update(chatMid, noteId, text)`, `delete(chatMid, noteId)`, `comments(chatMid, noteId, cursor)`, and `comment(chatMid, noteId, text)`. Resolve concrete cursor and transport types from task 1 evidence.

- [ ] Write fake-transport tests using sanitized response fixtures for valid group notes and comments, Thai text, error envelopes, missing author, invalid or repeated cursors, and preservation of rich content. Verify tests fail before the adapter exists.
- [ ] Implement normalized parsing and explicit server error handling. Validate chat and note IDs at the boundary; reject unsupported chats and writes with unknown ownership. Disable rich-note updates unless all original contents can be retained.
- [ ] Run `cd protocol && deno test --allow-read --allow-env tests/notes_test.ts` and confirm all adapter tests pass. Run `deno fmt --check src/notes.ts tests/notes_test.ts` and `deno lint src/notes.ts tests/notes_test.ts` from `protocol`.

### Task 3: Sidecar commands

**Files:** Modify `protocol/src/main.ts`; extend `protocol/tests/notes_test.ts` or existing command tests.

**Interfaces:** Commands `list_notes`, `get_note`, `create_note`, `update_note`, `delete_note`, `list_note_comments`, and `create_note_comment` accept `chatMid`, plus `noteId`, `text`, and `cursor` where relevant. Return the task 2 normalized models through existing `ok`/`fail` envelopes.

- [ ] Add fake-client tests for session absence, chat/ID validation, author mismatch, explicit group permission and `TALKROOM`, server errors, and one network call on a write timeout.
- [ ] Implement the commands with task 2's service and the existing authenticated client. Keep a request bound to its supplied chat and note IDs.
- [ ] Run `cd protocol && deno check src/main.ts` and the focused note tests. Confirm passing output and no real network write.

### Task 4: GTK notes experience

**Files:** Create `src/ui/notes.rs`; modify `src/protocol.rs`, `src/ui/shell.rs`, `src/ui/mod.rs`, `src/ui/state.rs`, `src/ui/events.rs`, and language JSON files.

**Interfaces:** Deserialize the normalized task 2 models in `src/protocol.rs`. Add `open_notes(state: &AppState, chat_mid: &str)` and dedicated pending variants carrying a weak window/session reference and immutable chat/note IDs.

- [ ] Add a fake-sidecar GTK fixture testing the header button, list, pagination, detail, create, own-note edit/delete confirmation, comment display/add, Thai labels, and error/retry states.
- [ ] Assert unsupported rooms are explained; rich-note edits are disabled when preservation is unverified; double clicks send one write; failed/ambiguous writes retain drafts; late responses do not update another chat or closed window; unknown author cannot edit/delete.
- [ ] Add the window and strings with loading, empty, ready, failed and exhausted states. Display author, time, text and attachment indicators. Send requests through task 3 and only mark writes successful after server confirmation.
- [ ] Run the focused GTK fixture with `GTK_A11Y=none GDK_DEBUG=no-portals cargo test <actual_fixture_name> -- --ignored --test-threads=1`; substitute the implemented fixture name and confirm it passes. Run `cargo test` and verify relevant failures rather than suppressing them.

### Task 5: Integrated verification and handoff

**Files:** All changed files; optional sanitized `.audit/notes-protocol/` evidence.

- [ ] Run `cd protocol && deno task check`, `cargo test`, and `cargo build --release`; record exact failures and any environmental limitations.
- [ ] Exercise the GTK fixture from the chat header through reading, writing, confirming deletion, adding comments and switching chats. Verify actual UI state and fake server requests.
- [ ] If an authenticated read-only check is available, verify listing against an actual group; do not create or delete real notes as an automated check. Distinguish mock coverage from live evidence.
- [ ] Review `git diff --check` and `git status --short`; retain the existing attachment edits. Report unsupported 1:1, attachment limitations and unverified server behavior accurately. Do not install, commit or publish unless the user later authorizes it.
