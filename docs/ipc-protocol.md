# IPC protocol (v1)

Local-only, newline-delimited JSON over a Unix domain socket or a Windows named
pipe. This protocol is for the desktop UI, CLI, and native-messaging host. It is
versioned independently of the app (currently `PROTOCOL_VERSION = 1`).

## Transport

| Platform | Address |
|---|---|
| Linux/macOS | `$XDG_RUNTIME_DIR/downpour/engine.sock` (directory mode 0700); fallback `$TMPDIR/downpour-$UID/engine.sock` |
| Windows | Per-user named pipe derived from the user SID |

Environment override `DOWNPOUR_SOCKET` is reserved for tests and portable mode.

## Framing

One JSON object per line, terminated by `\n`. Maximum frame size: **8 MiB**.
UTF-8 only. Unknown methods return an `unknown_method` error; they do not close
the connection.

## Handshake

The first client frame must be:

```json
{
  "id": 0,
  "method": "hello",
  "params": {
    "protocol_version": 1,
    "token": "<32-byte hex token>",
    "client": "ui"
  }
}
```

`client` is one of `ui`, `cli`, `native_host`. The token is generated per engine
start and written to `engine.token` (0600; owner-only DACL on Windows). A bad
token closes the connection. The engine replies:

```json
{ "id": 0, "result": { "protocol_version": 1, "engine": "0.1.0" } }
```

## Requests and responses

Request: `{ "id": <u64>, "method": "<string>", "params": <object> }`
Response: `{ "id": <u64>, "result": <value> }` or
`{ "id": <u64>, "error": { "code": "<snake_case>", "message": "<detail>" } }`.

`id` is chosen by the client and must be answered in the same order; the engine
may process concurrently but responses carry the id.

## Methods (planned)

| Method | Params | Result |
|---|---|---|
| `hello` | handshake above | engine info |
| `engine.info` | `{}` | `EngineInfo` |
| `download.add` | `AddDownloadRequest` | `DownloadSnapshot` |
| `download.list` | `{ "filter": "all\|active\|queued\|completed" }` | `DownloadSnapshot[]` |
| `download.get` | `{ "id": "<uuid>", "segments": true }` | snapshot with segments |
| `download.pause` | `{ "id": "<uuid>" }` | snapshot |
| `download.resume` | `{ "id": "<uuid>" }` | snapshot |
| `download.cancel` | `{ "id": "<uuid>" }` | snapshot |
| `download.remove` | `{ "id": "<uuid>", "delete_file": false }` | `{}` |
| `queue.list` | `{}` | queue summaries |
| `settings.get` | `{ "key": "..." }` | value |
| `settings.set` | `{ "key": "...", "value": <json> }` | value |
| `browser.add` | `BrowserContext` | snapshot (native host only) |
| `browser.media` | media candidates | `{}` (native host only) |

## Events

Server-pushed frames have no `id`:

```json
{ "event": "download.progress", "data": { ...DownloadSnapshot } }
```

| Event | Payload | Rate |
|---|---|---|
| `download.progress` | snapshot summary | coalesced 4–10 Hz |
| `download.completed` | snapshot | on completion |
| `download.failed` | snapshot + `ErrorInfo` | on failure |
| `queue.changed` | queue summary | on change |
| `engine.shutdown` | `{}` | on exit |

Clients must tolerate unknown events. Snapshot lists are never streamed at high
frequency; detail views request `download.get` and merge progress deltas.

## Error codes

Codes are shared with `downpour_types::ErrorCode`: `invalid_url`,
`unsupported_scheme`, `path_traversal`, `private_address`, `http`,
`auth_required`, `proxy_auth_required`, `range_unsupported`, `content_changed`,
`network`, `io`, `disk_full`, `checksum_mismatch`, `database`, `canceled`,
`not_implemented`, `internal`.

The UI translates codes; engines never send user-facing English.
