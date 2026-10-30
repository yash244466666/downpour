# Roadmap

Status as of 2026-10-30.

| Milestone | Scope | State |
|---|---|---|
| **M0** | Workspace, types/store, Tauri shell, dashboard preview, testkit skeleton, CI, docs, licenses | ✅ complete |
| **M1** | Single-stream engine: probe, temp file, SQLite journal, resume, pause, CLI | ✅ complete |
| **M2** | Multi-connection transfers, dynamic segmentation, throttle, retries, kill-9 crash tests | ✅ complete |
| **M3** | Full desktop UI wired to the engine: table, detail graph, settings, tray, notifications | ⏳ next |
| **M4** | Queue, priorities, categories/rules, scheduler, autostart, single-instance, updater wiring | ⬜ |
| **M5** | IPC server/client, native-messaging host, Chromium/Edge extension takeover | ⬜ |
| **M6** | yt-dlp integration, media detection UI, HLS fallback | ⬜ |
| **M7** | Site grabber + batch import/expansion | ⬜ |
| **M8** | Packaging: NSIS per-user, universal DMG, AppImage/deb/rpm; signing/notarization; release CI | ⬜ |
| **M9** | Hardening: accessibility, i18n, `downpour doctor`, docs | ⬜ |

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

## Next: M3 acceptance

- The dashboard lists real downloads from the engine (no mock data).
- Add, pause, resume, cancel, and remove work from the UI.
- Detail view with per-segment progress and speed.
- Settings persist to SQLite; theme and defaults survive restarts.
- Tray icon, notifications, and close-to-tray behavior.
- UI tests (Vitest) for formatting, filters, and dialog validation.

## Lean MVP cut line

If schedule pressure appears: M0–M5 Chromium-only + M6 yt-dlp-only. Defer the
grabber, scheduler, Firefox/Edge, native DASH, and i18n. That is still a credible
IDM-class v1.
