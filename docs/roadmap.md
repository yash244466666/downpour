# Roadmap

Status as of 2026-10-30.

| Milestone | Scope | State |
|---|---|---|
| **M0** | Workspace, types/store, Tauri shell, dashboard preview, testkit skeleton, CI, docs, licenses | ✅ complete |
| **M1** | Single-stream engine: probe, temp file, SQLite journal, resume, pause, CLI | ✅ complete |
| **M2** | Multi-connection planner, work-stealing, throttle, retries, kill-9 crash tests | ⏳ next |
| **M3** | Full desktop UI wired to the engine: table, detail graph, settings, tray, notifications | ⬜ |
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

Evidence: `cargo test --workspace` (77 tests) plus a CLI smoke run that paused
a 20 MiB download at ~36%, resumed it, and matched the server's SHA-256.

## Next: M2 acceptance

- Split range-capable transfers across up to 32 connections using the planner.
- Work-stealing: an early finisher takes the tail of the slowest segment.
- Global and per-download bandwidth throttling with a byte-rate token bucket.
- Retry policy hardening: per-segment retries, `Retry-After`, jitter cap.
- `kill -9` crash tests at randomized offsets, including around checkpoint and
  finalize boundaries; SHA-256 must match after resume.
- Concurrency across downloads is **not** part of M2; it lands with the queue.

## Lean MVP cut line

If schedule pressure appears: M0–M5 Chromium-only + M6 yt-dlp-only. Defer the
grabber, scheduler, Firefox/Edge, native DASH, and i18n. That is still a credible
IDM-class v1.
