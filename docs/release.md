# Releasing Downpour

## Versioning

1. Bump `version` in `apps/desktop/src-tauri/tauri.conf.json` and the
   workspace `Cargo.toml` `[workspace.package] version`.
2. Merge everything into `dev`, then move `main` onto `dev`.
3. Tag the release: `git tag v0.3.0 && git push origin v0.3.0`.

The `Release` workflow builds on Ubuntu, Windows, and macOS (universal), and
opens a **draft** GitHub release with the installers, `downpour` and
`downpour-nm-host` binaries, and the browser extension packages
(`downpour-chromium-<version>.zip`, `downpour-firefox-<version>.zip`). The
extension packages are built from `extensions/browser/build.mjs` and stamped
with this version, so there is nothing extra to bump by hand.

## Version bumps

Bump the version in all four places before tagging:

- `Cargo.toml` (`[workspace.package] version`)
- `apps/desktop/src-tauri/tauri.conf.json`
- `package.json`
- `apps/desktop/package.json`

Then run `cargo check --workspace` to refresh `Cargo.lock`.

The browser extension manifests are not a fifth place: `build.mjs` stamps
their version from `tauri.conf.json` when it assembles `dist/`. Keep the
checked-in manifests at the `0.0.0` placeholder.

## Artifact rules

The release workflow uploads **only** installers (`.deb`, `.AppImage`, NSIS
`.exe`, `.dmg`), the staged `downpour` / `downpour-nm-host` binaries, and the
packaged extension zips. Never upload the whole `bundle/` directory: the
AppImage `AppDir` contains hundreds of runtime shared libraries, which
previously polluted a release and failed the upload step. Never upload
`extensions/browser/dist/` itself; only the zips built from it.

## Local build

```bash
pnpm install
pnpm --dir apps/desktop tauri build --bundles deb
```

Linux bundling needs `patchelf`. macOS produces an `.app` + `.dmg`; Windows
produces an NSIS installer. All builds are unsigned unless the signing
credentials below are present.

## Signing and notarization

Provide these as repository secrets and uncomment the `env:` block in
`.github/workflows/release.yml`:

| Secret | Purpose |
|---|---|
| `APPLE_CERTIFICATE` | base64 `.p12` Developer ID Application certificate |
| `APPLE_CERTIFICATE_PASSWORD` | certificate password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | notarization credentials |
| `TAURI_SIGNING_PRIVATE_KEY` | updater signing key (see below) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | updater key password |

Do **not** pass empty secrets: Tauri would try to import an empty certificate
and fail the build. Unsigned builds are the default because the block is
commented out.

Windows Authenticode is not wired by default. Configure `bundle.windows` with
your `signCommand` and certificate, or sign the NSIS output in a release step.

## Updater

The updater plugin is compiled in and the Settings dialog can call
`check_for_updates`, but no update channel is configured. To enable it:

1. Generate a key pair:

   ```bash
   pnpm --dir apps/desktop tauri signer generate -w ~/.tauri/downpour.key
   ```

2. Set `TAURI_SIGNING_PRIVATE_KEY` and `..._PASSWORD` in CI.
3. Add to `apps/desktop/src-tauri/tauri.conf.json`:

   ```json
   "bundle": { "createUpdaterArtifacts": true },
   "plugins": {
     "updater": {
       "pubkey": "<public key>",
       "endpoints": ["https://example.com/downpour/{{target}}/{{arch}}/{{current_version}}"]
     }
   }
   ```

4. Publish `latest.json` and the signed bundles from the draft release.

The updater plugin is registered only when `plugins.updater` exists; without
it, startup is unaffected and `check_for_updates` returns "Updates are not
configured in this build". Do not add `plugins.updater` without a valid
`pubkey`, or plugin initialization will fail at startup.

## Browser native host

The release job uploads `downpour-nm-host` and the `downpour` CLI next to the
installers. After installing:

- Put `downpour-nm-host` at a stable path (or keep it beside the CLI).
- Run `downpour register-browsers` once per user.
- AppImage mounts at a new path each launch, so copy the host to
  `~/.local/share/downpour/bin/` before registering.

## Release history

- **v0.3.0** (2026-10-31) — live speed graph: the Total speed card and the
  sidebar throughput sparkline share a rolling 72-second history instead of
  hardcoded arrays.
- **v0.2.0** (2026-10-31) — Firefox download take-over; shared extension source
  tree with per-browser builds; extension zips attached to the same GitHub
  release; host/extension protocol gate; Firefox native-host registration and
  `doctor` coverage.
- **v0.1.2** (2026-10-31) — file-manager reveal fix for AppImages; bulk batch
  through the scheduler; immediate list rows; hidden-filter hints; clear
  finished; delete-file choice; categories-only sidebar; 2000-row list.
- **v0.1.0** (2026-10-31) — first release.
- v0.1.1 was tagged but superseded before publication.

## Release checklist

- [ ] CI green on `dev`.
- [ ] Version bumped in both places.
- [ ] Tag pushed.
- [ ] Draft release artifacts reviewed (NSIS, DMG, AppImage, deb, binaries).
- [ ] Signed/notarized builds when credentials are available.
- [ ] `docs/roadmap.md` and `README.md` updated.
