# Troubleshooting

Run the built-in diagnostics first:

```bash
downpour doctor
```

It checks the database, the download folder, yt-dlp/ffmpeg, IPC reachability,
and browser native-messaging manifests.

## The app shows "Preview data"

The browser-only preview (`pnpm --dir apps/desktop dev`) has no Tauri runtime,
so it keeps labelled mock rows. Launch the real app with `pnpm tauri dev` or an
installed build.

## "Engine offline" in the status bar

The status bar checks `engine_info` through Tauri. If it reports offline while
the window is the real app, the command bridge failed — check the devtools
console (`Ctrl+Shift+I` in dev) for a capability or CSP error.

## Browser downloads are not taken over

1. Start Downpour once; the app writes `engine.token` and hosts the IPC socket.
2. Register the native host: `downpour register-browsers`.
3. Load `extensions/browser/chromium` unpacked in Chrome/Edge.
4. Check `chrome://extensions` for the pinned ID
   (`lelbkegljeljlebagmalnnocmcmgpdjg`). If you repacked the extension, its ID
   may differ from the clipboard; regenerate the key or update the CLI constant.
5. AppImage builds mount at a new path each launch: copy `downpour-nm-host` to
   `~/.local/share/downpour/bin/` and re-run `register-browsers`.
6. Firefox and Safari are not shipped.

## `blob:` or `data:` download is ignored

Those URLs cannot cross native messaging; the extension leaves them alone so
the browser handles them.

## A download fails immediately

- HTTP 401/403: the server needs auth; cookies are only available through the
  browser handoff, and only for the current process.
- "server does not support ranges": the file downloads in one stream and cannot
  resume; restarting begins at byte zero.
- "server changed content": the ETag/Last-Modified changed; the transfer
  restarted.
- Disk full: free space and retry.

## Media downloads

- HLS uses yt-dlp when installed, otherwise the built-in fallback (highest
  variant only, AES-128 supported).
- DASH needs yt-dlp; the fallback does not mux DASH.
- SAMPLE-AES/Widevine/PlayReady are refused by design (no DRM circumvention).
- `DOWNPOUR_DISABLE_YTDLP=1` forces the built-in HLS path.

## "Open download folder" does nothing

Fixed in **v0.1.2**. The AppImage sets `LD_LIBRARY_PATH` to its bundled
libraries, which broke the system `xdg-open`. v0.1.2 uses the freedesktop
`FileManager1.ShowItems` call with the AppImage variables stripped (and falls
back to `gio`/`xdg-open`). Update to v0.1.2 or newer.

## Downloads are missing from the list

Status tabs, the category filter, and search all hide rows. The toolbar shows
**"Showing X of Y · Show all"** when that happens; click it to clear the
filters. The list is capped at 2000 rows; **Clear** removes finished rows
(files stay on disk). If the engine is unreachable, a red banner appears with a
Retry button.

## Batch import is slow or only some URLs were added

Fixed in **v0.1.2**: batch import is one bulk request and the scheduler starts
downloads within the concurrency/per-host limits. The toast reports the first
error if some URLs were rejected. The dialog expands at most 500 URLs; split
larger lists.

## I removed a download; where did the file go?

The trash action asks first: **Remove from list** keeps the file, **Delete
file** removes it. **Clear** only removes finished rows from the list.

## Linux window is blank or glitchy

Try:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm tauri dev
WEBKIT_DISABLE_COMPOSITING_MODE=1 pnpm tauri dev
```

Missing `libwebkit2gtk-4.1` or `libayatana-appindicator3` produces the same
symptom on some distributions.

## macOS says the app is damaged

Unsigned builds are quarantined by Gatekeeper. For local testing:
`xattr -dr com.apple.quarantine /Applications/Downpour.app`. Signed and
notarized releases avoid this.

## Windows SmartScreen

Unsigned NSIS builds trigger SmartScreen. Use "More info → Run anyway" for
local testing; releases should be Authenticode-signed.

## The grabber finds nothing

- robots.txt may disallow the paths.
- The default depth is 2 and pages 100; raise `--depth`/`--pages`.
- Same-origin is the default; use `--cross-origin` deliberately.
- File links are recognised by extension; pages without a media/archive
  extension are crawled, not queued.
