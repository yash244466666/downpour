# Roadmap

Status as of 2026-10-30.

| Milestone | Scope | State |
|---|---|---|
| **M0** | Workspace, types/store, Tauri shell, dashboard preview, testkit skeleton, CI, docs, licenses | ✅ complete |
| **M1** | Single-stream engine: probe, temp file, SQLite journal, resume, pause, CLI | ⏳ next |
| **M2** | Multi-connection planner, work-stealing, throttle, retries, kill-9 crash tests | ⬜ |
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

## Next: M1 acceptance

- Probe a URL and report size, range support, ETag, filename.
- Download a file over a single stream with a `.downpour-part` temp file.
- Persist state in SQLite; resume after a process kill without re-downloading
  completed bytes.
- Pause/resume/cancel from the CLI.
- Fault-injection tests: no-range, dropped connection, ETag change, 429,
  truncation, traversal filenames.
- Byte-exact output verified by SHA-256.

## Lean MVP cut line

If schedule pressure appears: M0–M5 Chromium-only + M6 yt-dlp-only. Defer the
grabber, scheduler, Firefox/Edge, native DASH, and i18n. That is still a credible
IDM-class v1.
