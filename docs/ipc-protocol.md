# IPC protocol (v1)

Local-only, newline-delimited JSON over a Unix domain socket or a Windows named
pipe. This protocol is for the desktop UI, CLI, and native-messaging host. It is
versioned independently of the app (currently `PROTOCOL_VERSION = 1`).

## Transport

| Platform | Address |
|---|---|
| Linux/macOS | `$XDG_RUNTIME_DIR/downpour/engine.sock` (directory mode 0700); fallback `~/.local/share/downpour/downpour/engine.sock` when `XDG_RUNTIME_DIR` is unset |
| Windows | Namespaced pipe `downpour-engine` |

The token file is `engine.token` in the per-user data directory (0600 on Unix).

## Framing

One JSON object per line, terminated by `\n`. Maximum frame size: **8 MiB**.
UTF-8 only. Unknown methods return an `unknown_method` error; they do not close
the connection.

## Handshake

The first client frame is a `Hello` object (not a request):

```json
{ "protocol_version": 1, "token": "<64-char hex token>", "client": "native_host" }
```

`client` is one of `ui`, `cli`, `native_host`. The token is generated per engine
start and written to `engine.token` (0600; owner-only DACL on Windows). A bad
token closes the connection. The engine does not send a handshake reply; the
client may send its first request immediately.

The client and server keep **one buffered connection** for the whole session.
Creating a fresh `BufReader` per frame would discard bytes the peer already sent.

## Requests and responses

Request: `{ "id": <u64>, "method": "<string>", "params": <object> }`
Response: `{ "id": <u64>, "result": <value> }` or
`{ "id": <u64>, "error": { "code": "<snake_case>", "message": "<detail>" } }`.

`id` is chosen by the client and must be answered in the same order; the engine
may process concurrently but responses carry the id.

## Methods (implemented)

| Method | Params | Result |
|---|---|---|
| `ping` / `engine.info` | `{}` | `EngineInfo` |
| `download.add` | `{ url, name?, directory?, connections?, start_now?, referer?, user_agent?, cookie_header?, headers? }` | `{ "id": "<uuid>" }` |
| `download.list` | `{}` | download views (same shape as the UI) |
| `download.pause` | `{ "id": "<uuid>" }` | `{ "ok": true }` |
| `download.resume` | `{ "id": "<uuid>" }` | `{ "ok": true }` |
| `download.cancel` | `{ "id": "<uuid>" }` | `{ "ok": true }` |
| `download.remove` | `{ "id": "<uuid>", "delete_file": false }` | `{ "ok": true }` |

`cookie_header` is split into cookies and passed to the engine as ephemeral
headers: they are used for this process only and never written to SQLite.
HLS/DASH URLs are routed to the media pipeline (yt-dlp when installed, native
HLS fallback otherwise); `cookie_header` is also written to a 0600 cookie file
for yt-dlp.

## Native-messaging host envelope

`downpour-nm-host` translates each browser message into one engine request and
wraps successful responses with two top-level fields:

```json
{
  "result": { "...": "engine result" },
  "extension_protocol": 1,
  "host_version": "0.3.3"
}
```

`extension_protocol` is the host↔extension compatibility version, independent
of both `PROTOCOL_VERSION` and the app version. Extensions compare it with
their own constant and leave the download to the browser when the host is too
old. A missing field means a legacy host and is treated as compatible, so old
and new sides roll out in either order. Error responses keep the existing
`{ "error": { "code", "message" } }` shape.

### Planned

`download.get` (per-segment detail), `queue.list`, `settings.get/set`,
`browser.media`, and server-pushed events. The UI currently polls
`download.list` at ~1 Hz.

## Events (planned)

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
