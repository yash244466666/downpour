//! Browser native-messaging host metadata and registration.
//!
//! Chromium browsers discover a native host through a per-user registry value
//! on Windows and through a per-browser configuration directory elsewhere;
//! Firefox uses its own directory. The CLI, `downpour doctor`, and the desktop
//! app all call into this module so they agree on the host name, the extension
//! IDs, and the registry hive.
//!
//! Registration is **per user**. On Windows the value is written under `HKCU`
//! explicitly: `reg.exe` falls back to `HKLM` when the root key is omitted,
//! which fails for an unelevated install and leaves Chrome unable to find the
//! host at all.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::paths;

/// Native-messaging host name shared by every browser.
pub const NM_HOST_NAME: &str = "com.downpour.nm";
/// Stable Chromium extension ID derived from the public key in
/// `extensions/browser/chromium/manifest.json`.
pub const CHROME_EXTENSION_ID: &str = "lelbkegljeljlebagmalnnocmcmgpdjg";
/// Stable Firefox add-on id; must match `extensions/browser/firefox/manifest.json`.
pub const FIREFOX_EXTENSION_ID: &str = "downpour@yash244466666.github.io";

/// Per-user Chromium registry keys, rooted at `HKCU` on purpose.
pub const CHROMIUM_REGISTRY_KEYS: [&str; 4] = [
    r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.downpour.nm",
    r"HKCU\Software\Chromium\NativeMessagingHosts\com.downpour.nm",
    r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.downpour.nm",
    r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.downpour.nm",
];

/// Per-user Firefox registry key on Windows.
pub const FIREFOX_REGISTRY_KEY: &str =
    r"HKCU\Software\Mozilla\NativeMessagingHosts\com.downpour.nm";

/// File name of the native host binary for this platform.
#[must_use]
pub fn host_binary_name() -> &'static str {
    if cfg!(windows) {
        "downpour-nm-host.exe"
    } else {
        "downpour-nm-host"
    }
}

/// Manifest handed to Chromium browsers.
#[must_use]
pub fn chromium_host_manifest(host: &Path) -> Value {
    json!({
        "name": NM_HOST_NAME,
        "description": "Downpour native messaging host",
        "path": host.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{CHROME_EXTENSION_ID}/")],
    })
}

/// Manifest handed to Firefox.
#[must_use]
pub fn firefox_host_manifest(host: &Path) -> Value {
    json!({
        "name": NM_HOST_NAME,
        "description": "Downpour native messaging host",
        "path": host.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID],
    })
}

/// Where the shared Chromium manifest is kept on Windows.
#[must_use]
pub fn manifest_path() -> PathBuf {
    paths::data_dir()
        .join("native-messaging")
        .join(format!("{NM_HOST_NAME}.json"))
}

/// Locate the native host next to an executable (sidecar/install layout).
#[must_use]
pub fn find_host_near(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let candidate = directory.join(host_binary_name());
    candidate.is_file().then_some(candidate)
}

/// Locate the native host for the current process.
///
/// `DOWNPOUR_NM_HOST` wins; otherwise the binary is expected beside the
/// running executable, which is where the installer and Tauri sidecars put it.
#[must_use]
pub fn find_host() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOWNPOUR_NM_HOST") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let executable = std::env::current_exe().ok()?;
    find_host_near(&executable)
}

/// One browser location inspected by [`inspect`].
#[derive(Debug, Clone)]
pub struct RegistrationEntry {
    /// Browser family label.
    pub browser: String,
    /// Manifest path reported by the browser directory or registry value.
    pub manifest: Option<PathBuf>,
    /// Native host path recorded in the manifest.
    pub host: Option<PathBuf>,
    /// True when the manifest exists and its host binary is present.
    pub ok: bool,
}

/// Result of a registration pass.
#[derive(Debug, Default)]
pub struct RegistrationReport {
    /// Manifests written (or registry values confirmed) during this pass.
    pub written: Vec<PathBuf>,
    /// Per-target failures; registration still succeeds if one browser fails.
    pub failures: Vec<String>,
}

impl RegistrationReport {
    /// True when nothing was registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.written.is_empty()
    }
}

/// Register `host` with every browser installed for the current user.
///
/// Idempotent: existing values are overwritten with the current host path.
/// Returns an error only when nothing at all could be registered.
pub fn register(host: &Path) -> io::Result<RegistrationReport> {
    if !host.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("native host not found: {}", host.display()),
        ));
    }

    let chromium =
        serde_json::to_vec_pretty(&chromium_host_manifest(host)).map_err(io::Error::other)?;
    let firefox =
        serde_json::to_vec_pretty(&firefox_host_manifest(host)).map_err(io::Error::other)?;

    #[cfg(windows)]
    {
        windows::register(&chromium, &firefox)
    }
    #[cfg(not(windows))]
    {
        unix::register(&chromium, &firefox)
    }
}

/// Inspect every browser location for a usable registration.
#[must_use]
pub fn inspect() -> Vec<RegistrationEntry> {
    #[cfg(windows)]
    {
        windows::inspect()
    }
    #[cfg(not(windows))]
    {
        unix::inspect()
    }
}

/// Directories a Chromium browser uses for its profile on Unices.
#[must_use]
pub fn browser_config_dirs_for(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support");
        vec![
            base.join("Google/Chrome"),
            base.join("Chromium"),
            base.join("Microsoft Edge"),
            base.join("BraveSoftware/Brave-Browser"),
        ]
    } else {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        vec![
            config.join("google-chrome"),
            config.join("chromium"),
            config.join("microsoft-edge"),
            config.join("BraveSoftware/Brave-Browser"),
        ]
    }
}

/// Firefox reads native-messaging manifests from a fixed per-user directory.
#[must_use]
pub fn firefox_manifest_dirs_for(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![home.join("Library/Application Support/Mozilla/NativeMessagingHosts")]
    } else {
        vec![home.join(".mozilla/native-messaging-hosts")]
    }
}

#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(not(windows))]
mod unix {
    use super::*;

    pub(super) fn register(chromium: &[u8], firefox: &[u8]) -> io::Result<RegistrationReport> {
        let mut report = RegistrationReport::default();
        let Some(home) = home_dir() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "HOME is not set; cannot find browser profiles",
            ));
        };

        for base in browser_config_dirs_for(&home) {
            if !base.exists() {
                continue;
            }
            let directory = base.join("NativeMessagingHosts");
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(format!("{NM_HOST_NAME}.json"));
            std::fs::write(&path, chromium)?;
            report.written.push(path);
        }

        for directory in firefox_manifest_dirs_for(&home) {
            if !directory.parent().is_some_and(Path::exists) {
                continue;
            }
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(format!("{NM_HOST_NAME}.json"));
            std::fs::write(&path, firefox)?;
            report.written.push(path);
        }

        if report.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no supported browser profile found; install Chrome, Chromium, Edge, \
                 Brave, or Firefox and retry",
            ));
        }
        Ok(report)
    }

    pub(super) fn inspect() -> Vec<RegistrationEntry> {
        let Some(home) = home_dir() else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for base in browser_config_dirs_for(&home) {
            let manifest = base
                .join("NativeMessagingHosts")
                .join(format!("{NM_HOST_NAME}.json"));
            entries.push(entry(browser_label(&base), manifest));
        }
        for directory in firefox_manifest_dirs_for(&home) {
            let manifest = directory.join(format!("{NM_HOST_NAME}.json"));
            entries.push(entry("Firefox".to_owned(), manifest));
        }
        entries
    }

    fn browser_label(base: &Path) -> String {
        let name = base
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| base.display().to_string());
        match name.as_str() {
            "Google/Chrome" | "google-chrome" => "Chrome".to_owned(),
            "chromium" | "Chromium" => "Chromium".to_owned(),
            "Microsoft Edge" | "microsoft-edge" => "Edge".to_owned(),
            "BraveSoftware/Brave-Browser" => "Brave".to_owned(),
            other => other.to_owned(),
        }
    }

    fn entry(browser: String, manifest: PathBuf) -> RegistrationEntry {
        let host = read_host(&manifest);
        RegistrationEntry {
            browser,
            manifest: Some(manifest),
            ok: host.as_deref().is_some_and(Path::is_file),
            host,
        }
    }

    fn read_host(manifest: &Path) -> Option<PathBuf> {
        let body = std::fs::read_to_string(manifest).ok()?;
        let value: Value = serde_json::from_str(&body).ok()?;
        value.get("path").and_then(Value::as_str).map(PathBuf::from)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    /// `CREATE_NO_WINDOW`: the GUI app must not flash a console per `reg` call.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Absolute path to `reg.exe`; never resolve through `PATH`.
    fn reg_exe() -> PathBuf {
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("reg.exe")
    }

    fn run_reg(
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> io::Result<std::process::Output> {
        Command::new(reg_exe())
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    }

    fn set_value(key: &str, manifest: &Path) -> Result<(), String> {
        let output = run_reg([
            OsStr::new("add"),
            OsStr::new(key),
            OsStr::new("/ve"),
            OsStr::new("/t"),
            OsStr::new("REG_SZ"),
            OsStr::new("/d"),
            manifest.as_os_str(),
            OsStr::new("/f"),
        ])
        .map_err(|error| format!("{key}: cannot run reg.exe: {error}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{key}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    fn query_value(key: &str) -> Option<PathBuf> {
        let output = run_reg([OsStr::new("query"), OsStr::new(key), OsStr::new("/ve")]).ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let value = text.lines().find_map(|line| {
            let (_, rest) = line.split_once("REG_SZ")?;
            let value = rest.trim();
            (!value.is_empty()).then_some(value)
        })?;
        Some(PathBuf::from(value))
    }

    pub(super) fn register(chromium: &[u8], firefox: &[u8]) -> io::Result<RegistrationReport> {
        let mut report = RegistrationReport::default();

        let shared = manifest_path();
        if let Some(parent) = shared.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&shared, chromium)?;
        report.written.push(shared.clone());

        let mut registered = 0_usize;
        for key in CHROMIUM_REGISTRY_KEYS {
            if query_value(key).as_deref() == Some(shared.as_path()) {
                registered += 1;
                continue;
            }
            match set_value(key, &shared) {
                Ok(()) => registered += 1,
                Err(error) => report.failures.push(error),
            }
        }

        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths::data_dir());
        let firefox_dir = appdata.join("Mozilla").join("NativeMessagingHosts");
        std::fs::create_dir_all(&firefox_dir)?;
        let firefox_manifest = firefox_dir.join(format!("{NM_HOST_NAME}.json"));
        std::fs::write(&firefox_manifest, firefox)?;
        if query_value(FIREFOX_REGISTRY_KEY).as_deref() != Some(firefox_manifest.as_path()) {
            match set_value(FIREFOX_REGISTRY_KEY, &firefox_manifest) {
                Ok(()) => report.written.push(firefox_manifest),
                Err(error) => report.failures.push(error),
            }
        }

        if registered == 0 {
            return Err(io::Error::other(format!(
                "no Chromium browser registry key could be written \
                 (a per-user install must not need administrator rights): {}",
                report.failures.join("; ")
            )));
        }
        Ok(report)
    }

    pub(super) fn inspect() -> Vec<RegistrationEntry> {
        let mut entries = Vec::new();
        for (browser, key) in [
            ("Chrome", CHROMIUM_REGISTRY_KEYS[0]),
            ("Chromium", CHROMIUM_REGISTRY_KEYS[1]),
            ("Edge", CHROMIUM_REGISTRY_KEYS[2]),
            ("Brave", CHROMIUM_REGISTRY_KEYS[3]),
        ] {
            entries.push(entry(browser.to_owned(), query_value(key)));
        }
        entries.push(entry(
            "Firefox".to_owned(),
            query_value(FIREFOX_REGISTRY_KEY),
        ));
        entries
    }

    fn entry(browser: String, manifest: Option<PathBuf>) -> RegistrationEntry {
        let host = manifest.as_deref().and_then(read_host);
        RegistrationEntry {
            browser,
            manifest,
            ok: host.as_deref().is_some_and(Path::is_file),
            host,
        }
    }

    fn read_host(manifest: &Path) -> Option<PathBuf> {
        let body = std::fs::read_to_string(manifest).ok()?;
        let value: Value = serde_json::from_str(&body).ok()?;
        value.get("path").and_then(Value::as_str).map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_manifest_lists_the_pinned_extension() {
        let manifest = chromium_host_manifest(Path::new("/opt/downpour-nm-host"));
        assert_eq!(manifest["name"], NM_HOST_NAME);
        assert_eq!(manifest["type"], "stdio");
        assert_eq!(
            manifest["allowed_origins"][0],
            format!("chrome-extension://{CHROME_EXTENSION_ID}/")
        );
    }

    #[test]
    fn firefox_manifest_uses_allowed_extensions() {
        let manifest = firefox_host_manifest(Path::new("/opt/downpour-nm-host"));
        assert_eq!(manifest["allowed_extensions"][0], FIREFOX_EXTENSION_ID);
        assert!(manifest.get("allowed_origins").is_none());
    }

    #[test]
    fn every_chromium_key_is_per_user() {
        for key in CHROMIUM_REGISTRY_KEYS {
            assert!(
                key.starts_with(r"HKCU\"),
                "registration must stay per user: {key}"
            );
            assert!(key.ends_with(NM_HOST_NAME), "unexpected key: {key}");
        }
        assert!(FIREFOX_REGISTRY_KEY.starts_with(r"HKCU\"));
    }

    #[test]
    fn host_binary_name_matches_the_platform() {
        if cfg!(windows) {
            assert_eq!(host_binary_name(), "downpour-nm-host.exe");
        } else {
            assert_eq!(host_binary_name(), "downpour-nm-host");
        }
    }

    #[test]
    fn finds_only_an_existing_host_beside_the_executable() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("downpour");
        std::fs::write(&executable, b"").unwrap();
        assert!(find_host_near(&executable).is_none());

        let host = directory.path().join(host_binary_name());
        std::fs::write(&host, b"").unwrap();
        assert_eq!(find_host_near(&executable), Some(host));
    }

    #[cfg(not(windows))]
    #[test]
    fn chromium_config_dirs_match_the_platform() {
        let dirs = browser_config_dirs_for(Path::new("/home/tester"));
        assert_eq!(dirs.len(), 4);
        if cfg!(target_os = "macos") {
            assert_eq!(
                dirs[0],
                Path::new("/home/tester/Library/Application Support/Google/Chrome")
            );
        } else {
            assert_eq!(dirs[0], Path::new("/home/tester/.config/google-chrome"));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn firefox_manifest_dir_matches_the_platform() {
        let dirs = firefox_manifest_dirs_for(Path::new("/home/tester"));
        assert_eq!(dirs.len(), 1);
        if cfg!(target_os = "macos") {
            assert_eq!(
                dirs[0],
                Path::new("/home/tester/Library/Application Support/Mozilla/NativeMessagingHosts")
            );
        } else {
            assert_eq!(
                dirs[0],
                Path::new("/home/tester/.mozilla/native-messaging-hosts")
            );
        }
    }
}
