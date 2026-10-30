# Downpour browser extension

Chromium MV3 extension that hands downloads to the local engine.

## Install (unpacked, development)

1. Build the native host and CLI:

   ```bash
   cargo build -p downpour-nm-host -p downpour-cli
   ```

2. Register the native-messaging manifest with installed Chromium browsers:

   ```bash
   ./target/debug/downpour register-browsers
   ```

3. Open `chrome://extensions` (or `edge://extensions`), enable Developer
   mode, choose **Load unpacked**, and select `extensions/browser/chromium`.

4. Start Downpour once so the IPC server writes `engine.token`.

Downloads from the browser are then cancelled and re-added to Downpour. If the
engine is offline, the browser's own download proceeds.

## Notes

- The extension ID is pinned by the `key` in `manifest.json`
  (`lelbkegljeljlebagmalnnocmcmgpdjg`), which must match the CLI's
  `CHROME_EXTENSION_ID`.
- `blob:` and `data:` URLs cannot be handed off.
- Cookies are passed to the engine for this process only; they are never
  written to SQLite.
- The native host binary must stay at a stable path. AppImage builds copy it to
  `~/.local/share/downpour/bin/` before registering.
