# Email/password login

- [x] 1. `how` over the affected subsystem.
- [x] 2. `architect` for parallel design exploration.
- [x] 3. Write the throughput checkpoint as four todo items.
  - [ ] **Blocking first steps.** Gates run before fan-out.
  - [ ] **Independent workstreams.** Disjoint files, services, or layers parallelize. Shared writes serialize.
  - [ ] **Shared mutable state.** Default to splitting the target. Serialize only for real invariants.
  - [ ] **Smallest safe decomposition.** If one worker is best, name why.
- [x] 4. Delegate code-writing to a subagent using your configured feature model with a specific scope.
- [x] 5. Verify on the matching surface.
- [x] 6. Rebase into small, ordered commits. Stack follow-ups.
- [x] 7. If the design is contested, `interrogate` before shipping.
- [x] 8. Run **Opening a PR**.

Data shape and design will be recorded after subsystem exploration.

Chosen data shape is LoginMethod and LoginPhase in one UI state plus login_password email/password IPC. Existing PIN and token/device contracts remain. Alternative is a protocol authentication coordinator with attempt IDs. Existing process restart cancellation makes the smaller design sufficient.

Throughput checkpoint.
- Blocking first steps completed by read-only UI/protocol explorers and locked linejs API plus Context7.
- Independent workstreams are design exploration and parent baseline verification, while one worker owns implementation.
- Shared mutable state stays isolated in /tmp/line-yux-email-login. No parent edits there.
- Smallest safe decomposition is one implementation owner because login lifecycle crosses UI, IPC and process cancellation.

Verification. Root cargo build passed. Rust unit suite 11 passed plus native GTK lifecycle test passed explicitly. Deno check passed with 9 tests. Actual GTK app fake peer checks passed, QR-error regression rerun pending. Standard clippy found existing HEAD src/tray.rs chunks_exact_mut warning on Rust 1.98. Clippy passed with only that legacy lint allowed. Independent review found QR auto-retry loop, fixed with typed Recovering phase and second independent review pending.

- 6. skip: deliver local uncommitted workspace changes, no commit requested.
- 7. skip: no contested design remains after independent review.
- 8. skip: gh auth status confirms no authenticated GitHub host. Deliver local reviewed changes without remote PR.

Final independent review passed. Actual GTK app QR error stayed idle until explicit Retry, then all email failure/PIN/cancellation checks passed again on final root build. No real LINE account credentials used. Probe script and output are /tmp/line-yux-login-probe/probe.py and probe.log. All product changes remain in the primary workspace.
