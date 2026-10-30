# Roadmap

Status as of 2026-10-31.

| Milestone | Scope | State |
|---|---|---|
| **M0** | Workspace, types/store, Tauri shell, dashboard preview, testkit skeleton, CI, docs, licenses | ✅ complete |
| **M1** | Single-stream engine: probe, temp file, SQLite journal, resume, pause, CLI | ✅ complete |
| **M2** | Multi-connection transfers, dynamic segmentation, throttle, retries, kill-9 crash tests | ✅ complete |
| **M3** | Full desktop UI wired to the engine: table, settings, tray, notifications | ✅ complete |
| **M4** | Queue, priorities, category filters, scheduler, autostart, single-instance | ✅ complete |
| **M5** | IPC server/client, native-messaging host, Chromium/Edge extension takeover | ✅ complete |
| **M6** | yt-dlp integration, media detection UI, HLS fallback | ✅ complete |
| **M7** | Site grabber + batch import/expansion | ✅ complete |
| **M8** | Packaging: deb/AppImage/NSIS/DMG, release workflow, signing hooks | ✅ complete |
| **M9** | Diagnostics, accessibility, docs (i18n deferred) | ✅ complete |

## Definition of done for M0

- [x] `cargo test --workspace` passes.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes (CI).
- [x] `cargo fmt --all --check` passes.
- [x] `pnpm --dir apps/desktop build` passes.
- [x] The Tauri window opens and renders the dashboard preview.
- [x] CI covers Linux, Windows, and macOS.
- [x] Architecture, IPC protocol, and ADRs are documented.

## Definition of done for M1

- [x] Probe a URL and report size, range support, ETag, and filename.
- [x] Download a file over a single stream with a `.downpour-part` temp file.
- [x] Persist state in SQLite; resume after an engine restart without
      re-downloading committed bytes (verified against the test server and by
      SIGINT/restart through the CLI).
- [x] Pause/resume/cancel through the CLI (`Ctrl-C` pauses; `downpour run <id>`
      resumes; `downpour cancel <id>` deletes the partial file).
- [x] Fault-injection tests: no-range, dropped connection, ETag change,
      mid-transfer content change, 429 + `Retry-After`, permanent truncation,
      redirects, auth failure, and traversal filenames.
- [x] Byte-exact output verified by SHA-256 in every transfer test.

Evidence: `cargo test --workspace` (78 tests) plus a CLI smoke run that paused
a 20 MiB download at ~36%, resumed it, and matched the server's SHA-256.

## Definition of done for M2

- [x] Range-capable transfers split across up to 32 workers.
- [x] Dynamic segmentation: a shared queue finer than the worker count, so an
      early finisher pulls another range (no idle connections).
- [x] Global and per-download byte-rate throttling with a token bucket.
- [x] Per-segment retries with `Retry-After` and jittered backoff.
- [x] A real `SIGKILL` mid-transfer test that resumes to a byte-exact file.
- [x] Multi-connection observed at the server: several distinct range starts.

Concurrency across downloads is **not** part of M2; it lands with the queue.

## Definition of done for M3

- [x] The dashboard lists real downloads from the engine (mock data only in the
      browser-only preview, labelled).
- [x] Add, pause, resume, cancel, remove, and reveal-in-folder work from the UI
      (verified against a live Tauri window).
- [x] Live progress and speed while downloading.
- [x] Settings persist to SQLite and survive restarts.
- [x] Tray icon and close-to-tray behavior; completion notifications.
- [x] 12 Vitest unit tests for formatting and record mapping.

## Definition of done for M4

- [x] Persistent queue with concurrency and per-host limits.
- [x] Priorities with up/down controls on queued rows.
- [x] Category filters in the sidebar.
- [x] Start/stop schedule window (including overnight windows) with weekday
      selection; running transfers pause outside it.
- [x] Start all / Pause all; the scheduler starts queued items only.
- [x] Autostart toggle and single-instance focus.
- [x] Verified live: five queued downloads with a limit of three produce
      exactly three active and two queued; Pause all and Start all behave.
- [ ] Updater wiring — deferred to M8 with release signing keys.
- [ ] Category path rules (per-category folders) — filters shipped; rules are
      future work.

## Definition of done for M5

- [x] Local IPC server and client with a per-start token (0600 token file).
- [x] `downpour register-browsers` writes Chromium/Edge/Brave manifests with a
      pinned extension ID.
- [x] The native host validates browser argv and forwards add/list/pause/
      resume/cancel/remove to the engine, auto-launching the app if needed.
- [x] A Chromium/Edge MV3 extension takes over downloads and hands over
      cookies, referer, and user agent; blob:/data: URLs are left alone.
- [x] End-to-end verified: native host → IPC → engine add/list completed an
      8 MiB download; cookies were not persisted to SQLite.
- [ ] Firefox/Safari extensions — not shipped.

## Definition of done for M6

- [x] HLS/DASH/progressive URLs are classified; the extension sniffs manifests.
- [x] yt-dlp is detected (system install) and runs with a 0600 cookie file,
      progress parsing, and process-tree cancellation.
- [x] The built-in HLS fallback picks the highest variant, downloads and
      concatenates segments, decrypts AES-128, and refuses SAMPLE-AES.
- [x] Media downloads appear in the dashboard and complete with a checksum.
- [x] Verified end to end: HLS master playlist through the native host
      completed byte-exact (3,072 bytes) with yt-dlp disabled.
- [ ] Managed yt-dlp auto-download (pinned SHA-256) — not shipped; system
      detection only.
- [ ] User-facing quality selection — highest variant only.

## Definition of done for M7

- [x] Robots-aware BFS crawler with depth/page caps and per-host delay.
- [x] Link extraction from `<a>`, `<img srcset>`, video/audio/source, and CSS
      `url()`; forms are treated as links but never submitted.
- [x] Same-origin by default; files are filtered through robots.txt too.
- [x] Persisted `grabber_jobs` and `grabber_frontier` rows.
- [x] CLI `grab` lists found files and `--add` queues them.
- [x] Batch import dialog with pattern expansion, dedupe, preview, and a 500
      URL cap.
- [ ] Grabber UI (the current UI has batch import, not a crawl dialog).

## Definition of done for M8

- [x] Bundle metadata: publisher, category, descriptions, per-user NSIS,
      macOS 13 minimum.
- [x] `tauri build --bundles deb` produces a working 9.4 MiB package with the
      right dependencies, binary, and icons.
- [x] Release workflow builds Linux (deb, AppImage), Windows (NSIS), and
      universal macOS bundles, plus CLI/native-host binaries, and opens a draft
      release.
- [x] Signing/notarization env hooks documented (empty secrets must not be
      passed).
- [x] Updater plugin compiled and `check_for_updates` wired; no channel/key.
- [ ] Windows and macOS installers verified on this machine (CI-only).
- [ ] RPM target (dropped from the workflow to keep it reliable).

## Definition of done for M9

- [x] `downpour doctor` checks the database, folders, tools, IPC, and browser
      manifests.
- [x] Visible keyboard focus; reduced-motion support; dialog autofocus.
- [x] Troubleshooting, accessibility, and release docs.
- [x] i18n status documented honestly (English-only; `ui.language` reserved).

## Post-v1 backlog

- Signed/notarized release builds (credentials required).
- Updater channel, signing key, and `latest.json` publication.
- Managed yt-dlp download with pinned SHA-256; HLS quality selection.
- Firefox and Safari extensions; grabber UI.
- Bengali localization and a full string table.
- Dialog focus traps and automated axe/WCAG checks.
- Optional `cargo deny`/`cargo audit` CI once license/advisory policy is set.

## Lean MVP cut line

If schedule pressure appears: M0–M5 Chromium-only + M6 yt-dlp-only. Defer the
grabber, scheduler, Firefox/Edge, native DASH, and i18n. That is still a credible
IDM-class v1.
