# CLAUDE.md — Downpour

> Project context for Claude Code. Read this before changing anything.
> Last updated: 2026-10-31 (M9 complete).

## 1. What this is

**Downpour** is a cross-platform (Windows 10/11, macOS 13+, Linux) download
manager in the IDM class: 1–32 parallel connections per file, crash-safe resume,
queues, scheduler, bandwidth throttling, browser download take-over, media
grabbing (HLS/DASH/yt-dlp), and a site grabber — wrapped in a modern dashboard.

**Current status: M9 complete.** All v1 milestones are delivered except the
documented gaps: signed/notarized releases, a configured update channel,
managed yt-dlp download, quality selection, Firefox/Safari extensions, the
grabber UI, and Bengali localization. Do not describe those as shipped.

| Area | State |
|---|---|
| Rust workspace | Compiles; 117 tests pass; clippy `-D warnings` clean |
| Shared types / IPC contract | DTOs, error codes, framing, handshake types |
| SQLite schema + migrations | Initial schema, WAL, STRICT tables, download queries |
| Segment planner / filename safety | Planner exists but drives M2; filename hardening live |
| Transfer engine | Probe/transfer/resume/pause/finalize, single-stream and segmented |
| Multi-connection transfer | 1–32 workers over a dynamic segment queue (M2) |
| Throttling | Per-download and global byte-rate token bucket |
| Crash testing | Engine restart tests + a real SIGKILL CLI test (Unix) |
| IPC server/client | Local socket, token auth, add/list/pause/resume/cancel/remove |
| Native host | Validates browser argv, forwards to IPC, auto-launches the app |
| Browser extension | Chromium/Edge MV3 take-over + context menu |
| CLI | `add`, `get`, `run`, `list`, `info`, `pause`, `cancel`, `version` work |
| Desktop UI | **Live** dashboard; settings, tray, notifications, categories |
| Queue / scheduler | Concurrency + per-host caps, priorities, schedule window |
| Startup | Autostart toggle and single-instance focus |
| Frontend tests | 20 Vitest unit tests (formatting, filters, mapping) |
| Media | HLS parser/downloader, AES-128, yt-dlp detection/run; DASH via yt-dlp |
| Grabber | Robots-aware BFS crawler, batch expansion, persisted job/frontier |
| Packaging | `.deb` verified locally; release workflow for NSIS/DMG/AppImage |
| Diagnostics | `downpour doctor` (DB, folders, tools, IPC, browser manifests) |
| Accessibility | Focus rings, reduced motion, labelled controls, dialog autofocus |
| Signing / updater | Hooks documented; updater plugin wired, unconfigured |

If your task touches an unimplemented area, the milestone that owns it is in
[§11](#11-milestones) and `docs/roadmap.md`.

## 2. Read these first

1. `docs/architecture.md` — process model, pipeline, crash safety.
2. `docs/ipc-protocol.md` — wire format, methods, events, error codes.
3. `docs/roadmap.md` — milestone state and M1 acceptance criteria.
4. `docs/adr/0001-stack.md` and `docs/adr/0002-writer-thread.md` — decisions
   that are not up for casual revision.
5. `AGENT.md` — working agreement, branch protocol, definition of done.

## 3. Non-negotiable invariants

1. **The engine has no Tauri/UI dependency.** `downpour-core` must remain usable
   from a headless daemon. UI concerns never enter it.
2. **One writer thread per download owns the only file handle.** Never
   `try_clone()` (Unix duplicates and Windows duplicates both share the file
   offset), never share a `tokio::fs::File` with `seek` + `write`, never mmap.
   Segment tasks send `(offset, Bytes)` over a bounded channel.
3. **Checkpoint ordering:** `sync_data()` the file **first**, then commit
   segment offsets to SQLite in one transaction. A durable DB offset must never
   be ahead of durable bytes.
4. **Trust the response, not the header.** Range support is proven by a real
   `206` + valid `Content-Range`; `Accept-Ranges: bytes` alone means nothing.
   `Accept-Encoding: identity` is mandatory for byte-range work.
5. **No multi-connection for no-range servers.** Do not fake `.part1/.part2`.
   Use one stream and show "server does not support ranges".
6. **Server-provided filenames are hostile.** Sanitize reserved Windows names,
   illegal characters, trailing dots/spaces, control characters, NTFS ADS
   colons; reject `..` and absolute paths; canonicalize the destination parent
   and verify the final path stays under it.
7. **Secrets never reach logs.** No tokens, cookies, `Authorization`, or signed
   URLs in logs, error messages, or crash reports. Persisted secrets use the OS
   keyring; cookies are ephemeral by default.
8. **No DRM circumvention and no paywall bypass.** HLS/DASH with SAMPLE-AES,
   Widevine, or PlayReady must be refused. `ffmpeg` is detected, never bundled.
   `yt-dlp` is optional, user-initiated, checksum-verified.
9. **Tauri 2 stable only.** Do not adopt the Tauri 3 alpha. Keep capabilities
   minimal; no `shell`, `fs`, `http`, or `sql` plugins — the engine owns those.
10. **The UI never touches SQLite, the filesystem, HTTP, or processes.** All
    actions go through Tauri commands / engine IPC. Error codes come from Rust;
    the UI translates them.

## 4. Repository map

```
downpour/
├─ Cargo.toml                  workspace, resolver 3, edition 2024
├─ crates/
│  ├─ downpour-types/          serde DTOs, ErrorCode, PROTOCOL_VERSION
│  ├─ downpour-store/          SQLite open/config/migrations
│  ├─ downpour-core/           engine (probe, transfer, writer, resume, planner)
│  ├─ downpour-ipc/            JSON-line protocol + framing
│  ├─ downpour-media/          media classification, yt-dlp/ffmpeg detection
│  ├─ downpour-grabber/        batch pattern expansion (crawler TODO)
│  ├─ downpour-nm-host/        browser native-messaging binary (framing only)
│  └─ downpour-testkit/        fault-injecting range server
├─ apps/
│  ├─ cli/                     `downpour` CLI (add/get/run/list/info/pause/cancel)
│  └─ desktop/                 Tauri 2 app + React dashboard
│     ├─ src/                  React 19 + Vite + Tailwind sources
│     └─ src-tauri/            Rust shell, tauri.conf.json, capabilities
├─ extensions/browser/         planned WXT extension (empty)
├─ docs/                       architecture, protocol, ADRs, roadmap, assets
└─ .github/workflows/ci.yml    3-OS Rust gates + frontend gates
```

Key files:

| Concern | File |
|---|---|
| Protocol DTOs | `crates/downpour-types/src/lib.rs` |
| SQLite schema | `crates/downpour-store/src/migrations.rs` |
| Download queries | `crates/downpour-store/src/downloads.rs` |
| Probe + filename parsing | `crates/downpour-core/src/probe.rs` |
| Engine orchestration | `crates/downpour-core/src/engine.rs` |
| Writer thread | `crates/downpour-core/src/writer.rs` |
| Cancellation | `crates/downpour-core/src/cancel.rs` |
| Segment math | `crates/downpour-core/src/planner.rs` |
| Filename safety | `crates/downpour-core/src/filename.rs` |
| IPC framing | `crates/downpour-ipc/src/protocol.rs` |
| Test HTTP server | `crates/downpour-testkit/src/lib.rs` |
| Native host framing | `crates/downpour-nm-host/src/main.rs` |
| Tauri command | `apps/desktop/src-tauri/src/lib.rs` |
| Dashboard entry | `apps/desktop/src/App.tsx` |
| Design tokens | `apps/desktop/src/styles.css` |

## 5. Commands

```bash
# Rust — run from repo root
cargo test --workspace
cargo test -p downpour-core planner          # one module
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo fmt --all --check

# Frontend — run from repo root
pnpm install                                 # pnpm 10; lockfile committed
pnpm --dir apps/desktop dev                  # browser preview, mock data
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop build
pnpm tauri dev                               # real Tauri window (needs DISPLAY)

# CLI (engine runs in-process in M1)
cargo run -p downpour-cli -- version
cargo run -p downpour-cli -- add <URL> -n name.ext
cargo run -p downpour-cli -- run <ID>         # Ctrl-C pauses, rerun resumes
cargo run -p downpour-cli -- list
cargo run -p downpour-cli -- info <ID>
cargo run -p downpour-cli -- cancel <ID>      # only when not running
cargo run -p downpour-cli -- register-browsers # Chromium/Edge native messaging
cargo run -p downpour-cli -- doctor            # environment diagnostics

# Useful
cargo check -p downpour-desktop              # Tauri shell only
sqlite3 <db> .schema                         # inspect the schema
```

**Expected local timings:** first `cargo check -p downpour-desktop` ~1–2 min;
first `pnpm tauri dev` ~4–6 min because the binary is linked with debuginfo;
subsequent runs are seconds. Do not interpret the first build as a hang.

## 6. Architecture in one screen

```
Browser extension (MV3) ──native messaging──► downpour-nm-host
                                                   │ JSON-line IPC
                                                   ▼
Tauri app (React dashboard) ◄──commands/events── downpour-core engine
                                                   │
                                        rusqlite (WAL) + .downpour-part files
```

Pipeline: **probe → plan → transfer → checkpoint → finalize**.

- Probe: `GET Range: bytes=0-0`, `Accept-Encoding: identity`, manual redirects
  (max 10), capture strong ETag / Last-Modified, final URL, filename.
- Plan: connections `clamp(1,32)`; one stream for no-range/unknown/<2 MiB.
  M1 always runs a single stream; the planner drives M2.
- Transfer: one network task per segment → bounded channel → single writer
  thread → positional writes.
- Checkpoint: fsync file, then DB transaction; reconcile on startup with
  `If-Range`.
- Finalize: verify length, optional SHA-256, atomic rename, fsync parent dir.

Progress events are coalesced at 4–10 Hz. Never emit one event per chunk.

## 7. Data model (initial migration)

`downloads`, `segments`, `queues`, `schedules`, `settings`, `history`,
`media_jobs`, `grabber_jobs`, `grabber_frontier`.

- Statuses: `queued`, `probing`, `downloading`, `paused`, `finalizing`,
  `completed`, `failed`, `canceled`.
- Segment states: `pending`, `active`, `done`, `failed`.
- Timestamps are milliseconds UTC (`INTEGER`).
- IDs are UUIDv7 text.
- Tables are `STRICT`; status columns have `CHECK` constraints.
- `headers_enc` is for keyring-encrypted sensitive headers only.

## 8. Version pins and why

| Pin | Reason |
|---|---|
| Rust edition 2024, MSRV 1.85 | Workspace policy; local toolchain is newer |
| `rusqlite` **0.40** | `rusqlite_migration` 2.6 depends on 0.40; 0.41 conflicts on `libsqlite3-sys` links |
| `rusqlite_migration` 2.6 | Latest; pairs with rusqlite 0.40 |
| `reqwest` 0.13 | Features are `rustls`, `rustls-native-certs` — there is **no** `rustls-tls` feature in 0.13 |
| `axum` 0.8 | Route params use `{size}`, **not** `:size` |
| Tauri 2.x (`tauri` 2.11, CLI 2.12) | Stable line; Tauri 3 is alpha and unsupported here |
| React 19 / Vite 8 / Tailwind 4 / TS 7 | Current frontend stack; Tailwind is CSS-first (`@theme`) |
| pnpm 10 | `packageManager` field and lockfile |

Adding or changing a pin? Update this table in the same commit.

## 9. Coding standards

### Rust

- Edition 2024, `rustfmt` default, clippy `-D warnings`.
- `thiserror` for public errors; no `unwrap`/`expect` on fallible paths in
  library code (tests may unwrap).
- `#[must_use]` on pure constructors/queries.
- Public items get doc comments. Module-level docs explain the invariant, not
  the syntax.
- Prefer plain functions over traits until a second implementation exists.
- Log with `tracing`; never log secrets. Use structured fields.
- Avoid `std::process::Command` without absolute paths, argument arrays (no
  shell), and process-tree cleanup.

### TypeScript / React

- `strict`, `noUnusedLocals`, `noUnusedParameters`; no `any`.
- Named exports; components under `src/components`, pure helpers under `src/lib`.
- Tailwind semantic tokens (`bg-surface`, `text-muted`, `border-line`,
  `text-accent`). Raw hex only inside SVG assets.
- Every interactive control needs an accessible name; dialogs need
  `role="dialog"`, `aria-modal`, and Escape handling.
- Do not encode status by color alone; pair icon/text.
- Mock data is clearly labelled `Preview data`. Never present it as live.

### Copy

Sentence case, no exclamation marks, no marketing adjectives in UI. Error
messages say what happened and what the user can do.

## 10. Testing and quality gates

**Definition of gate-clean:**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test
pnpm --dir apps/desktop build
```

- Engine correctness is defined by `downpour-testkit`, not live internet tests:
  `/ok`, `/no-range`, `/versioned`, `/flip-after-range`, `/drop`,
  `/always-truncated`, `/retry-429`, `/disposition`, `/redirect`, `/auth`
  exist today; add `/gzip-range`, `/slow`, and a TCP cut proxy as needed.
- Use `proptest` for planner tiling invariants.
- Crash tests use a spawned CLI process + SIGKILL (`taskkill /F` on Windows)
  and compare SHA-256 against the source.
- UI tests currently stop at typecheck/build; Vitest + Playwright land with M3.
- Current total: **117 Rust tests** (42 core, 22 engine integration, 1 CLI
  crash, 6 store, 6 testkit, 4 types, 6 IPC transport/protocol, 11 media,
  11 grabber + 2 crawl integration, plus native-host units) and **20 frontend
  tests** (Vitest).
- Never add a test that downloads from the public internet.

## 11. Milestones

Full detail in `docs/roadmap.md`.

| # | Scope | State |
|---|---|---|
| M0 | Workspace, schema, planner, filename, IPC types, testkit, dashboard, CI, docs | ✅ |
| M1 | Probe, single-stream transfer, SQLite journal/resume, pause, CLI | ✅ |
| M2 | Multi-connection, dynamic segmentation, throttle, retries, kill-9 tests | ✅ |
| M3 | Full UI wired to engine, tray, notifications, settings | ✅ |
| M4 | Queue, scheduler, categories, autostart, single-instance | ✅ |
| M5 | IPC server/client, native host, Chromium/Edge extension | ✅ |
| M6 | yt-dlp, media detection, HLS fallback | ✅ |
| M7 | Site grabber, batch import | ✅ |
| M8 | Packaging, signing, notarization, release CI | ✅ |
| M9 | Accessibility, diagnostics, docs | ✅ |

**M1 exit criteria (met):** a real file downloads over one stream; state
persists; kill the process and resume without re-downloading committed bytes;
CLI pause/resume; fault tests pass; output SHA-256 matches.

**M2 exit criteria (met):** range-capable files split across up to 32 workers
pulling from a dynamic segment queue; byte-rate throttling; per-segment
retries; a real SIGKILL mid-transfer resumes to a byte-exact file.

**M3 exit criteria (met):** the dashboard shows live downloads from the engine,
add/pause/resume/cancel work from the UI, settings persist, and the tray and
notifications are wired.

**M4 exit criteria (met):** a persistent queue with priorities and per-host
limits, category filters, a start/stop scheduler, autostart, and
single-instance. The updater is deferred to M8, where release signing keys
exist; the updater UI is not shipped yet.

**M5 exit criteria (met):** a local IPC server/client with token auth, the
native-messaging host registered by the CLI, and a Chromium/Edge extension that
takes over downloads and hands over cookies, referer, and user agent. Firefox
and Safari extensions are not shipped.

**M6 exit criteria (met):** media URLs are detected (HLS/DASH/progressive),
yt-dlp is detected and runs with cookie handoff, and the built-in HLS fallback
downloads a stream (highest variant, AES-128, SAMPLE-AES refused). Managed
yt-dlp auto-download and user-facing quality selection are **not** shipped;
DASH requires yt-dlp.

**M7 exit criteria (met):** a robots-aware, rate-limited site grabber with
depth and page caps, link extraction, a persisted frontier, and batch URL
import with pattern expansion and preview.

**M8 exit criteria (met):** a Linux `.deb` builds locally; the release
workflow builds Linux, Windows, and universal macOS bundles plus CLI/native
host artifacts; signing/notarization env hooks are documented and gated on
maintainer credentials. Windows and macOS installers are produced by CI, not
verified on this machine. The updater plugin is wired but has no update
channel or signing key.

**M9 exit criteria (met):** `downpour doctor`, visible keyboard focus and
reduced motion, accessibility notes, troubleshooting/release docs, and an
honest list of remaining gaps (i18n deferred).

**Post-v1 backlog:** signed/notarized releases, updater channel/keys, managed
yt-dlp download with pinned hashes, HLS quality selection, Firefox/Safari
extensions, grabber UI, Bengali localization (the `ui.language` setting is
reserved), dialogs focus trap, automated axe/CSP hardening.

## 12. Branching and releases

- **`dev` is the default branch.** All feature work branches from `dev` and
  merges back via PR.
- **`main` is release-only and pulls from `dev`.** Local `main` is configured to
  track `origin/dev`, so `git pull` on `main` brings dev's changes. Do not
  develop on `main`; do not force-push either branch.
- Branch names: `feat/<slug>`, `fix/<slug>`, `docs/<slug>`, `chore/<slug>`.
- Commits: imperative subject, ≤72 chars, body explains why. Include the
  attribution line required by the session.
- Version bumps happen only on release merges into `main`.

## 13. Security and legal checklist

Before committing anything that touches downloads, browser handoff, media, or
the grabber:

- [ ] URL schemes restricted to `http`/`https`.
- [ ] Private/link-local targets require explicit user confirmation.
- [ ] Destination path canonicalized; traversal and symlink TOCTOU considered.
- [ ] No secrets in logs, errors, fixtures, or screenshots.
- [ ] Spawned tools use absolute paths, no shell, and process-tree kill.
- [ ] `yt-dlp` downloads verify a pinned SHA-256.
- [ ] No DRM bypass; refuse SAMPLE-AES/Widevine/PlayReady.
- [ ] Site grabber defaults: robots.txt obeyed, per-host delay, page/byte caps.
- [ ] `cargo deny`/`cargo audit` clean once those configs land (M9).

## 14. Known traps

- `extensions/browser/chromium` is plain MV3 JavaScript, not a pnpm project;
  `pnpm-workspace.yaml` still lists `extensions/browser` and pnpm tolerates the
  missing `package.json`. Firefox/Safari loaders are not shipped.
- Tauri capability schema (`apps/desktop/src-tauri/gen/schemas/...`) is generated
  at build time and gitignored; editors may show it as missing until first build.
- `pnpm tauri dev` requires a display; CI runs `cargo check/test` only.
- One `Engine` runs **one download at a time** (many connections per download).
  The desktop app gives each running download its own engine/connection; the
  scheduler bounds concurrency.
- `Engine::open` performs crash recovery once per process. Any later engine
  (per Tauri command, per download, CLI `list`/`pause`/`cancel`) must use
  `open_without_recovery`, or it will flip a live transfer to `paused`. Only
  CLI `run`/`get` recover.
- The IPC handshake and every later frame must use **one** buffered connection
  per client; a fresh `BufReader` per frame loses bytes already sent.
- Browser cookies and handoff headers are ephemeral (in-memory only). Never
  persist them; the SQLite `headers_enc` keyring path is still TODO.
- The Chromium extension ID (`CHROME_EXTENSION_ID` in the CLI) must match the
  `key` in `extensions/browser/chromium/manifest.json`.
- Media jobs update the same `downloads` rows but are not engine transfers:
  pausing kills the process and resuming restarts it from zero. HLS/DASH URLs
  are routed by `queue_add`; DASH needs yt-dlp. `DOWNPOUR_DISABLE_YTDLP=1`
  forces the built-in HLS path.
- Media tasks must not hold a SQLite `Store` across an await (the future must
  stay `Send`); open it for short updates only.
- The Rust `tauri` crate and `@tauri-apps/api` must share the same 2.x minor;
  `tauri build` refuses mismatches. The API is pinned to 2.11.x for the 2.11
  crate.
- Release signing must only pass non-empty secrets; empty `APPLE_CERTIFICATE`
  makes the bundler try to import an empty certificate.
- The grabber runs in the CLI (not the UI yet). It honours robots.txt and a
  per-host delay by default, filters discovered files through robots too, and
  stores the canonical URL in `grabber_frontier.url_hash` (no hashing).
- Batch import caps expansion at 500 URLs in the UI and 10,000 in the pattern
  expander.
- The desktop scheduler ticks every 1.5 s, starts **queued** downloads only
  (never paused ones) within concurrency/per-host limits, and pauses running
  transfers outside the schedule window. Do not auto-start paused rows; that
  would override an explicit user pause.
- Segments are finer-grained than workers (about 3×), and workers pull from a
  shared queue; do not reintroduce per-worker fixed segments.
- Ctrl-C in the CLI pauses (commits progress, keeps the `.downpour-part`).
  `downpour pause <id>` works in-process; cross-process pause uses the IPC
  methods (`download.pause`) through the app or native host.
- A no-range download always restarts from byte zero; never write a restarted
  body at a non-zero offset.
- Linux WebKitGTK rendering glitches: try
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` or `WEBKIT_DISABLE_COMPOSITING_MODE=1`.
- Changing `tauri.conf.json` or capabilities triggers a Rust rebuild.
- The CLI binary is named `downpour`; the crate is `downpour-cli` (package) and
  the desktop crate is `downpour-desktop`.
- `Cargo.lock` is committed deliberately (application, not library).
- Do not commit `target/`, `node_modules/`, `dist/`, or generated Tauri schemas.
- Screenshots belong in `docs/assets/` and should stay under ~500 KB.

## 15. When in doubt

- Prefer the existing pattern in the same crate over a new abstraction.
- Update the relevant doc/ADR when a decision changes; do not leave stale docs.
- If a task requires credentials, code signing, destructive filesystem work, or
  a scope decision (drop Firefox, change licensing), stop and ask in chat — do
  not guess.
