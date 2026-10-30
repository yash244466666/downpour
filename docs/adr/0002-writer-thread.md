# ADR 0002 — One writer thread per download

- **Status:** accepted
- **Date:** 2026-10-30

## Context

Segmented downloads have up to 32 workers writing into one file at different
offsets. The obvious implementations are all subtly wrong:

- Sharing a `tokio::fs::File` and calling `seek` + `write` lets workers race on
  the file cursor.
- `File::try_clone()` duplicates the file description on Unix and the handle on
  Windows; both share the file offset, so concurrent seeks still race.
- Memory-mapping the destination turns truncation into `SIGBUS` and does not
  provide crash-safe append semantics.

## Decision

Each download owns exactly one `std::fs::File` on a dedicated writer thread.
Segment tasks send `(absolute_offset, Bytes)` over a bounded channel. The writer
performs `write_all_at` on Unix and a looping `seek_write` on Windows.

## Consequences

- No cursor races and a single place to fsync and checkpoint.
- Bounded channels provide backpressure; a fast network cannot exhaust memory.
- One writer thread per active download is cheap relative to network workers.
- Checkpoints must `sync_data()` before committing offsets to SQLite, otherwise
  a durable database offset can be ahead of durable file bytes.
- The writer thread must not call async APIs; it drains a receiver and exits
  when all senders are dropped.
