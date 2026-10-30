# Architecture

Downpour is a Rust engine with a Tauri 2 shell, a local IPC bridge, a browser
extension, and a native-messaging host. This document describes the pieces and
the invariants that keep them safe.

## Process model (v1)

```
┌──────────────────────────────┐          ┌──────────────────────────────────┐
│ Browser extension (MV3)      │          │ Downpour desktop (Tauri 2)       │
│ Chromium / Firefox           │          │  React dashboard (webview)       │
│ · media sniffing             │          │  Tauri commands ↑ / events ↓     │
│ · download take-over         │          │  ┌────────────────────────────┐  │
└──────────────┬───────────────┘          │  │ downpour-core engine       │  │
               │ native messaging         │  │ tokio · reqwest · planner  │  │
┌──────────────▼───────────────┐          │  └───────────┬────────────────┘  │
│ downpour-nm-host             │          │              │                   │
└──────────────┬───────────────┘  local   │  SQLite (rusqlite)   .part files │
               └──── JSON-line IPC ──────►│                                  │
                 (UDS / named pipe)       └──────────────────────────────────┘
```

A single tray-resident process owns the engine. Closing the window hides it;
downloads continue. The engine crate has no Tauri dependency, so a future
version can move it into a daemon without rewriting the transfer code.

## Crate responsibilities

| Crate | Owns | Must not own |
|---|---|---|
| `downpour-types` | serde DTOs, error codes, protocol version | I/O, DB, HTTP |
| `downpour-store` | schema, migrations, SQL | engine policy |
| `downpour-core` | probing, planning, transfer, checkpoints, throttling | UI, IPC, Tauri |
| `downpour-ipc` | framing, envelopes, handshake | transport policy |
| `downpour-media` | yt-dlp management, HLS parsing, tool detection | generic downloads |
| `downpour-grabber` | crawling, link extraction, batch expansion | file writing |
| `downpour-nm-host` | stdio framing, forwarding, app auto-launch | engine logic, DB |
| `downpour-testkit` | fault-injecting server, TCP cut proxy | production use |
| `apps/desktop` | UI, Tauri commands, tray | HTTP/file/DB logic |
| `apps/cli` | headless control + test harness | alternate engine |

## Transfer pipeline

1. **Probe** — `GET` with `Range: bytes=0-0` and `Accept-Encoding: identity`.
   Trust a real `206` + `Content-Range`, never `Accept-Ranges`. A `200` means
   single stream. Capture strong ETag / Last-Modified and the post-redirect URL.
2. **Plan** — clamp connections to 1–32; one segment for no-range, unknown-size,
   or sub-2 MiB files; otherwise split into ~3× `connections` segments (1 MiB
   minimum, 128 maximum) that exactly tile the file. Finer segments let workers
   pull new ranges instead of idling.
3. **Transfer** — one network task per segment; a bounded channel feeds a single
   writer thread that owns the only file handle and performs positional writes
   (`write_all_at` / `seek_write`). Do not clone handles: duplicated handles
   share the file offset on both Unix and Windows.
4. **Checkpoint** — `sync_data()` the file, then commit offsets to SQLite in one
   transaction. A durable offset must never be ahead of durable bytes.
5. **Finalize** — fsync, length check, optional SHA-256, atomic rename, fsync
   the parent directory on Unix, then mark complete.

### Implemented in M1–M4

`probe`, the single writer thread, checkpointing, resume, pause, finalize, the
CLI, segmented transfer, and throttling are live in `downpour-core` /
`apps/cli`.

- Segmented transfers spawn `min(connections, segments)` workers that pull
  segment indices from a shared queue. Each worker streams one `Range` request
  at a time; early finishers take the next range.
- The writer tracks a durable end offset per segment. A checkpoint syncs once
  and commits every segment in one transaction, so per-segment resume never
  trusts bytes past the last sync.
- A single-segment resume that receives `200` (content changed) truncates and
  restarts once the probe confirms; a multi-segment transfer treats a
  mid-flight `200` as a content change for the next run.
- Throttling is a token bucket shared by the transfer's workers, applied to
  both the segmented and single-stream paths.

One `Engine` drives one download at a time; the desktop app gives each running
download its own engine (and SQLite connection), so the UI stays responsive.
Queue-level concurrency arrives in M4. Every run re-probes, so a short-lived
signed URL is refreshed before each resume.

### Queue and scheduler (M4)

The queue lives in SQLite (`downloads.priority`, `status`). The desktop app
runs a scheduler tick every 1.5 s: it starts `queued` downloads in priority
order while respecting the global concurrency limit and a per-host cap, and
pauses running transfers outside the configured start/stop window. Paused rows
are never auto-started, so an explicit user pause wins. The CLI runs its queue
sequentially in one process.

### Desktop shell (M3)

Tauri commands are thin: `list_downloads`, `add_download`, `start_download`,
`pause_download`, `cancel_download`, `remove_download`, `get_settings`,
`set_settings`, and `reveal_path`. Each command opens a short-lived engine with
`Engine::open_without_recovery`; crash recovery runs exactly once at app
startup. The webview polls the list at ~1 Hz and receives progress events for
in-process transfers. A browser-only Vite preview keeps mock data and labels it
"Preview data".

### Crash safety

- `.downpour-part` lives beside the destination for an atomic same-filesystem
  rename.
- Startup moves `downloading`/`probing`/`finalizing` rows to `paused`. On resume
  the temp file is truncated to the last committed offset, so uncommitted tail
  bytes from a crash are never trusted.
- The probe result is compared with the stored validator before transferring.
  A changed ETag/Last-Modified discards committed bytes and restarts from zero;
  a `200` response to a range request (server changed or ignored `If-Range`)
  truncates the temp file and restarts the same way, capped at two restarts.
- A no-range server is always re-fetched from byte zero; the engine never fakes
  segments or writes a restarted body at the old offset.
- Orphan part files go to a repair list; nothing is silently deleted except an
  empty temp left under a superseded name.
- SQLite uses WAL + `synchronous=NORMAL`: process crashes are safe; power loss
  can lose the last checkpoint, which only causes re-downloading.

## Persistence

`rusqlite` with the `bundled` feature (no system SQLite dependency), migrations
via `rusqlite_migration`, one writer connection on a dedicated thread. Settings
live in SQLite and are only mutated through engine commands; the UI never talks
to the database.

## IPC (implemented)

- Unix domain socket at `$XDG_RUNTIME_DIR/downpour/engine.sock` (directory
  0700) or the namespaced pipe `downpour-engine` on Windows.
- Newline-delimited JSON, 8 MiB frame cap. One buffered connection per client;
  the handshake and all later frames share it.
- A 32-byte token generated per app start, written to a 0600 file. The first
  frame is a `Hello` carrying it; a bad token closes the connection.
- Implemented methods: `ping`/`engine.info`, `download.add`, `download.list`,
  `download.pause`, `download.resume`, `download.cancel`, `download.remove`.
- Browser cookies and handoff headers are ephemeral: they live in the engine's
  memory for the process and are never written to SQLite.

See [ipc-protocol.md](ipc-protocol.md).

## Media (M6)

Media URLs are classified in `downpour-media`. HLS master playlists choose the
highest-bandwidth variant; media playlists are downloaded segment by segment
with AES-128 decryption (SAMPLE-AES and unknown key methods are refused — no
DRM). yt-dlp is detected on `PATH` and, when present, handles HLS/DASH with
cookies in a 0600 Netscape file and process-group cancellation. DASH without
yt-dlp fails with a clear message. Media jobs reuse the `downloads` rows and
report progress through the same UI; they are not engine transfers, so pause
stops and resume restarts them.

## Browser integration

The native host is intentionally small: frame stdio, validate the caller,
forward to the engine, exit on EOF. Chromium passes the extension origin as
`argv[1]`; Firefox passes the manifest path and add-on id. Host→browser messages
are capped at 1 MB, so only commands and compact summaries cross the boundary.

Extension download take-over is `downloads.onCreated` → ping the engine →
`cancel()` → `erase()` → hand off URL + cookies + referer + UA. If the engine is
offline the browser download proceeds normally. A brief failed-download flash
and a leftover partial file are unavoidable; `blob:` and `data:` URLs cannot be
handed off at all. The native host validates Chrome's extension-origin argv and
Firefox's manifest-path argv, forwards one message at a time, and never writes
diagnostics to stdout.

## Security boundaries

- Untrusted input: server headers, filenames, HTML for the grabber, browser
  messages, media playlists.
- Filenames are sanitized per OS (reserved names, illegal characters, `..`,
  length) and the final path is verified under the chosen directory.
- Extension-initiated adds require a user gesture; private/link-local addresses
  need confirmation.
- Cookies/tokens are never logged; persisted secrets use the OS keyring.
- yt-dlp is downloaded only over HTTPS with a pinned SHA-256 and executed from
  an app-owned directory. ffmpeg is detected, never bundled.

## Platform notes

- **Windows:** WebView2, NSIS per-user installer, HKCU native-messaging keys.
  `set_len` preallocation is skipped in v1.
- **macOS:** WKWebView, universal app, DMG, notarization requires an Apple
  Developer account. No App Sandbox for direct distribution.
- **Linux:** WebKitGTK 4.1 is a runtime dependency for deb/rpm; AppImage mounts
  at a different path each launch, so the native host is copied to a stable
  `~/.local/share/downpour/bin/` location before registration.
