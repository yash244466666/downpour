//! Per-OS locations for the database and the default download folder.
//!
//! Deliberately dependency-free: `std::env` plus the XDG user-dirs file is
//! enough for the three desktop platforms.

use std::path::PathBuf;

fn home_dir() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable).map(PathBuf::from)
}

/// Directory that holds the database and other per-user state.
#[must_use]
pub fn data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        return home_dir()
            .map(|home| home.join("Library/Application Support/downpour"))
            .unwrap_or_else(|| PathBuf::from(".downpour"));
    }

    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join("AppData/Local")))
            .map(|base| base.join("downpour"))
            .unwrap_or_else(|| PathBuf::from(".downpour"));
    }

    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".local/share")))
        .map(|base| base.join("downpour"))
        .unwrap_or_else(|| PathBuf::from(".downpour"))
}

/// Default database file.
#[must_use]
pub fn database_path() -> PathBuf {
    data_dir().join("downpour.db")
}

/// Default download folder.
///
/// On Linux this honours `XDG_DOWNLOAD_DIR` and `~/.config/user-dirs.dirs`;
/// elsewhere it falls back to `~/Downloads`.
#[must_use]
pub fn default_download_dir() -> PathBuf {
    if cfg!(unix) && !cfg!(target_os = "macos") {
        if let Some(dir) = xdg_download_dir() {
            return dir;
        }
    }
    home_dir()
        .map(|home| home.join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(unix)]
fn xdg_download_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_DOWNLOAD_DIR") {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Some(path);
        }
    }

    let home = home_dir()?;
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let contents = std::fs::read_to_string(config_dir.join("user-dirs.dirs")).ok()?;

    for line in contents.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("XDG_DOWNLOAD_DIR=") else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if value.is_empty() {
            return None;
        }
        let expanded = value.replace("$HOME", &home.to_string_lossy());
        let path = PathBuf::from(expanded);
        return path.is_absolute().then_some(path);
    }
    None
}

#[cfg(not(unix))]
fn xdg_download_dir() -> Option<PathBuf> {
    None
}
