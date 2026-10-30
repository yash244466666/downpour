# ADR 0001 — Rust engine + Tauri 2 shell

- **Status:** accepted
- **Date:** 2026-10-30

## Context

Downpour needs to run on Windows, macOS, and Linux, perform many concurrent
range requests, write into one file safely, survive crashes, and expose a
modern UI. The candidate stacks were Electron + Node, Qt 6 + C++, Go + Wails,
and Rust + Tauri 2.

## Decision

Use a Rust workspace for the engine and a Tauri 2 desktop shell with a React +
TypeScript frontend.

## Why

- Segmented downloading is I/O-bound and crash-sensitive; Rust gives memory
  safety without a garbage collector and makes positional file I/O and
  cancellation straightforward.
- Tauri uses the OS webview, producing ~10–20 MB installers and low idle memory,
  while still allowing a modern UI.
- The engine can be built and tested as a standalone crate, then embedded in the
  app. A future daemon split does not require a rewrite.
- Electron's Node HTTP stack is weaker at high-concurrency ranged transfers and
  ships a larger runtime. Qt is fully native but slower to build across three
  platforms. Go is viable but has a smaller ecosystem for media parsing and
  desktop integration.

## Consequences

- Linux requires WebKitGTK 4.1 at runtime; Windows requires WebView2
  (preinstalled on Windows 10/11); macOS uses WKWebView.
- Tauri 2 is the supported line. Do not adopt the Tauri 3 alpha for v1.
- The UI must not own business logic: files, HTTP, SQLite, and processes stay in
  Rust. This keeps the engine reusable and the attack surface smaller.
- Installer size and memory targets are part of the definition of done.

## Rejected alternatives

- **Electron + TypeScript:** fastest UI iteration, but ~150 MB installers, high
  RAM, and a weaker transfer engine.
- **Qt 6 + C++:** excellent native feel, highest cross-platform plumbing cost.
- **Go + Wails:** good concurrency, fewer media/HTTP edge-case libraries and
  weaker desktop integrations.
