# Downpour browser extension

One shared extension source, assembled for each browser. The extension hands
downloads to the local engine through the native-messaging host.

| Browser | State |
|---|---|
| Chrome / Edge / Brave (Chromium MV3) | supported |
| Firefox (MV3 event page) | supported |
| Safari | not shipped — needs macOS and Xcode (issue #33) |

## Build

```bash
node extensions/browser/build.mjs
```

This writes `extensions/browser/dist/chromium/` and `dist/firefox/`, stamping
each manifest with the app version from
`apps/desktop/src-tauri/tauri.conf.json`. The checked-in manifests carry a
`0.0.0` placeholder; never edit their version by hand.

Validation and tests (no npm dependencies):

```bash
node extensions/browser/build.mjs --check
node --test extensions/browser/test/*.test.mjs
```

## Install (unpacked, development)

1. Build the native host and CLI:

   ```bash
   cargo build -p downpour-nm-host -p downpour-cli
   ```

2. Register the native-messaging manifests with installed browsers:

   ```bash
   ./target/debug/downpour register-browsers
   ```

   This writes Chromium/Edge/Brave manifests and the Firefox manifest
   (`~/.mozilla/native-messaging-hosts/` on Linux,
   `~/Library/Application Support/Mozilla/NativeMessagingHosts/` on macOS,
   the per-user `%APPDATA%\Mozilla\NativeMessagingHosts` plus registry key on
   Windows).

3. Chromium: open `chrome://extensions` (or `edge://extensions`), enable
   Developer mode, choose **Load unpacked**, and select
   `extensions/browser/dist/chromium`.

   Firefox: open `about:debugging#/runtime/this-firefox`, choose
   **Load Temporary Add-on**, and select `extensions/browser/dist/firefox/manifest.json`.

4. Start Downpour once so the IPC server writes `engine.token`.

Downloads from the browser are then cancelled and re-added to Downpour. If the
engine is offline, the browser's own download proceeds.

## Firefox signing

The GitHub release attaches the Firefox package, but Firefox stable refuses
permanently installed unsigned add-ons. The build loads as a temporary add-on
(until Firefox restarts). Permanent installation for normal users requires
signing through addons.mozilla.org; that mirrors the same version rather than a
separate release.

## Notes

- The Chromium extension ID is pinned by the `key` in
  `chromium/manifest.json` (`lelbkegljeljlebagmalnnocmcmgpdjg`), which must
  match the CLI's `CHROME_EXTENSION_ID`.
- The Firefox add-on id is `downpour@yash244466666.github.io`, which must match
  the CLI's `FIREFOX_EXTENSION_ID` and the native host's
  `allowed_extensions`.
- `blob:` and `data:` URLs cannot be handed off.
- Cookies are passed to the engine for this process only; they are never
  written to SQLite.
- The native host binary must stay at a stable path. AppImage builds copy it to
  `~/.local/share/downpour/bin/` before registering.
- The host reports an `extension_protocol` value in every successful response.
  An extension that sees an older host leaves downloads with the browser instead
  of breaking them.

## Layout

```
extensions/browser/
├─ shared/lib.js           pure helpers (URL eligibility, filenames, cookies)
├─ shared/background.js    listeners and native messaging
├─ chromium/manifest.json  MV3 service worker
├─ firefox/manifest.json   MV3 event page
├─ build.mjs               assemble, stamp, package
└─ test/                   node --test suites
```
