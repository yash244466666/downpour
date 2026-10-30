# AGENT.md — Working agreement for coding agents

> Audience: autonomous or semi-autonomous coding agents (Claude Code, Codex,
> similar). `CLAUDE.md` is the project knowledge base; this file is the operating
> procedure. When they disagree, `CLAUDE.md`'s invariants win.

## 0. Mission and current state

Ship Downpour milestone by milestone. Each milestone must leave the repository
gate-clean and honest about what works.

- **Current:** M2 complete (2026-10-31). Multi-connection transfers over a
  dynamic segment queue, throttling, per-segment retries, and SIGKILL resume.
- **Next:** M3 — wire the dashboard to the engine, tray, notifications,
  settings. Acceptance criteria are in `docs/roadmap.md`.
- **Not implemented:** IPC server, browser extension, media pipeline, crawler,
  packaging. Do not claim otherwise in code, docs, commits, or chat.

## 1. Bootstrap (every session)

Run these before touching code:

```bash
git status --short
git branch --show-current
git fetch origin --prune
git log --oneline -5

cargo test --workspace
pnpm --dir apps/desktop typecheck
```

Stop and report if:

- the tree is dirty with someone else's changes,
- the branch is `main` (switch to `dev` or a feature branch),
- the baseline gates fail for reasons unrelated to your task.

Read, in order: `CLAUDE.md` §3, `docs/roadmap.md`, the ADRs, and the protocol
document relevant to your area.

## 2. Branch and commit protocol

- `dev` is the default and integration branch.
- `main` is release-only and pulls from `dev`. Never develop on `main`.
- Create `feat/<slug>`, `fix/<slug>`, `docs/<slug>`, or `chore/<slug>` from the
  latest `dev`.
- Open a PR into `dev`. Do not merge your own PR unless the task explicitly says
  to; the maintainer merges.
- Never force-push `dev` or `main`. Never rewrite shared history.
- Commits are small and logical. Subject imperative, ≤72 characters; body
  explains *why*, not *what*.
- End commit messages and PR descriptions with the attribution lines required
  by the session instructions.
- Do not commit secrets, signing keys, certificates, tokens, or generated build
  output (`target/`, `node_modules/`, `dist/`).

## 3. Task intake

Before implementation, write down (in the PR draft or a scratch note):

```
Milestone:      M?
Deliverable:    one sentence
Acceptance:     observable criteria (commands, bytes, UI state)
Out of scope:   what you will not do
Touched areas:  crates/apps/files
Risks:          crash safety, security, platform-specific behavior
```

If acceptance criteria are ambiguous and the ambiguity changes the design,
**ask in chat** rather than guessing.

## 4. Implementation protocol

1. **Read the invariants** in `CLAUDE.md` §3. Re-read the ones your task touches.
2. **Reproduce or characterize** the current behavior. For engine work, add a
   route or fixture to `downpour-testkit` first.
3. **Write a failing test** for the smallest correct behavior. Planner/filename
   logic gets unit + property tests; I/O gets an integration test against the
   testkit.
4. **Implement the smallest slice** that makes the test pass. No speculative
   abstractions.
5. **Run the full gates** (below).
6. **Self-review the diff.** Look for: shared file handles, unwrapped errors,
   secret leakage, path traversal, missing cancellation, blocking calls on the
   async runtime, event storms.
7. **Update docs** if behavior or a decision changed: `docs/architecture.md`,
   `docs/ipc-protocol.md`, an ADR, and the roadmap checkbox.
8. **Commit and open the PR** with evidence (commands + output summary).

## 5. Quality gates (all must pass)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop build
```

For UI changes, also run the app:

```bash
pnpm tauri dev
```

and verify the actual window (not only the Vite browser preview). Capture a
screenshot for user-visible changes when practical.

Rules:

- No new clippy warnings. No `#[allow(...)]` without a comment explaining why.
- No `unwrap`/`expect` in library paths. Tests may unwrap.
- New public API needs a doc comment and a test.
- Do not weaken an existing assertion to make a test pass.
- Do not add a live-internet test.

## 6. Definition of done

- [ ] Acceptance criteria demonstrated.
- [ ] Gates green locally.
- [ ] New behavior covered by tests (unit + integration where relevant).
- [ ] Docs/roadmap/ADR updated if needed.
- [ ] No secrets or generated artifacts in the diff.
- [ ] Security checklist below satisfied.
- [ ] PR description includes: what changed, why, how it was verified, and any
      follow-ups or known gaps.
- [ ] Claims match reality. If part of the milestone is deferred, say so
      explicitly.

## 7. Area ownership map

| Area | Owns | Typical files |
|---|---|---|
| Types/protocol | `downpour-types` | `src/lib.rs` |
| Persistence | `downpour-store` | `src/lib.rs`, `src/migrations.rs` |
| Engine | `downpour-core` | `src/planner.rs`, `src/filename.rs`, future `src/engine/*`, `src/io/*` |
| IPC | `downpour-ipc` | `src/protocol.rs`, future transport |
| Media | `downpour-media` | `src/lib.rs`, future `src/ytdlp.rs`, `src/hls.rs` |
| Grabber | `downpour-grabber` | `src/lib.rs`, future `src/crawler.rs` |
| Native host | `downpour-nm-host` | `src/main.rs` |
| Test infrastructure | `downpour-testkit` | `src/lib.rs` |
| Desktop shell | `apps/desktop/src-tauri` | `src/lib.rs`, `tauri.conf.json`, `capabilities/` |
| Dashboard | `apps/desktop/src` | `App.tsx`, `components/`, `lib/` |
| CI/docs | `.github/`, `docs/`, root markdown | |

When a change crosses areas, keep the engine change in its own commit so it can
be reviewed and reverted independently.

## 8. Debug playbooks

### Dependency resolution / build failure

- Check the version table in `CLAUDE.md` §8 first. Most failures are known pins:
  - `reqwest 0.13` has no `rustls-tls` feature — use `rustls` +
    `rustls-native-certs`.
  - `rusqlite` must stay 0.40 while `rusqlite_migration` 2.6 requires it.
  - `axum 0.8` path params are `{size}`, not `:size`.
- `cargo tree -i <crate>` shows who pulls a conflicting version.
- Do not "fix" a version conflict by adding a second SQLite-linked crate.

### Engine / transfer bugs

- Reproduce against `downpour-testkit`, never a live URL.
- Log request/response status, `Content-Range`, expected vs actual offsets.
  Never log cookies or full URLs containing tokens.
- Verify the plan tiles `[0, total)` exactly before blaming the network layer.
- If bytes are corrupted, check for shared file handles or a checkpoint written
  before `sync_data`.

### Tauri window problems

1. Run the browser preview (`pnpm --dir apps/desktop dev`) to isolate UI vs shell.
2. Run `cargo check -p downpour-desktop` to isolate Rust.
3. Blank window: open devtools (`Ctrl+Shift+I` in dev) and check the console; a
   CSP or capability error appears there.
4. Linux rendering glitches: try `WEBKIT_DISABLE_DMABUF_RENDERER=1` or
   `WEBKIT_DISABLE_COMPOSITING_MODE=1`.
5. No display in the environment: report it; do not claim the app ran.

### SQLite / migrations

- Never edit `MIGRATION_001` after it has shipped. Add `MIGRATION_002`.
- Test both fresh open and re-open paths.
- WAL files (`-wal`, `-shm`) are normal; do not delete them while the engine runs.
- `PRAGMA integrity_check` is the first diagnostic for corruption reports.

### CI failures

- Reproduce locally with the same command. If it is OS-specific, state which
  runner and paste the failing output.
- Linux Tauri deps come from the workflow; if you add a crate needing a system
  library, update the workflow in the same PR.

## 9. Stop and escalate

Stop and ask in chat when:

- Credentials, code signing, notarization, store accounts, or paid services are
  required.
- A change would delete or overwrite user data or run destructive filesystem
  operations.
- Legal/ToS ambiguity appears (media grabbing, site crawling, DRM).
- A product decision is needed: scope cuts, licensing, telemetry, UI direction.
- The same error survives three focused attempts; report what you tried.
- A task conflicts with an invariant in `CLAUDE.md` §3.

## 10. Security and legal checklist

- [ ] Only `http`/`https` schemes accepted; reject `file:`, `data:`,
      `javascript:`.
- [ ] Private/link-local destinations require explicit confirmation.
- [ ] Destination canonicalized; traversal, ADS colons, and symlink TOCTOU
      considered.
- [ ] No secrets in logs, errors, fixtures, screenshots, or commit history.
- [ ] Cookies ephemeral by default; persisted secrets encrypted via keyring.
- [ ] External tools spawned with absolute paths, no shell, process-tree kill.
- [ ] `yt-dlp` downloads verify a pinned SHA-256; never execute unverified files.
- [ ] No DRM bypass (SAMPLE-AES/Widevine/PlayReady refused).
- [ ] Grabber honors robots.txt, per-host delay, and caps by default.
- [ ] `ffmpeg` is never bundled or auto-downloaded.

## 11. Common failure modes (avoid these)

1. Claiming an unimplemented feature works because the UI mock shows it.
2. Giving each segment its own cloned file handle.
3. Checking `Accept-Ranges` instead of the actual `206` response.
4. Forgetting `Accept-Encoding: identity` on probes and range requests, or
   retrying a no-range transfer at the old offset instead of truncating and
   restarting from zero.
5. Committing offsets to SQLite before `sync_data()`.
6. Emitting a progress event per chunk instead of coalescing.
7. Storing cookies in plaintext or logging request headers.
8. Editing an already-shipped migration.
9. Adding a second SQLite-linked crate or upgrading rusqlite casually.
10. Testing against the public internet instead of `downpour-testkit`.
11. Using `unwrap()` in a library path and calling it "temporary".
12. Pushing directly to `dev` or `main`, or force-pushing either.

## 12. Handoff template

When ending a work session, leave the next agent:

```
State:          milestone, branch, commit SHA
Works:          what is verified and how
Broken/TODO:    exact files + failing command
Next step:      the single next action
Risks:          anything unsafe to touch
Evidence:       commands run and their result
```

Put durable project knowledge in `CLAUDE.md`/`docs/`, not in chat. Chat history
is not part of the repository.
