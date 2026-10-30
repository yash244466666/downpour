//! Media detection and managed external tools.
//!
//! yt-dlp (Unlicense) may be downloaded as an optional component; ffmpeg is
//! detection-only because it is never bundled.

use std::path::{Path, PathBuf};

/// Paths to optional external tools discovered on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPaths {
    pub yt_dlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
}

/// Kind of media URL detected by the extension or a page scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Hls,
    Dash,
    Progressive,
}

/// Classify a URL by its path extension (query and fragment ignored).
#[must_use]
pub fn classify_url(url: &str) -> Option<MediaKind> {
    let stem = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();

    if stem.ends_with(".m3u8") {
        return Some(MediaKind::Hls);
    }
    if stem.ends_with(".mpd") {
        return Some(MediaKind::Dash);
    }

    const PROGRESSIVE: &[&str] = &[
        ".mp4", ".webm", ".mkv", ".mov", ".m4v", ".ts", ".m4s", ".mp3", ".m4a", ".flac", ".wav",
        ".ogg", ".opus",
    ];
    PROGRESSIVE
        .iter()
        .any(|ext| stem.ends_with(ext))
        .then_some(MediaKind::Progressive)
}

/// Detect system `yt-dlp` and `ffmpeg`.
#[must_use]
pub fn detect_tools() -> ToolPaths {
    ToolPaths {
        yt_dlp: find_in_path("yt-dlp"),
        ffmpeg: find_in_path("ffmpeg"),
    }
}

/// Find an executable on `PATH`.
#[must_use]
pub fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{binary}.exe"),
            format!("{binary}.cmd"),
            binary.to_owned(),
        ]
    } else {
        vec![binary.to_owned()]
    };

    for dir in std::env::split_paths(&path) {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_hls_dash_and_progressive() {
        assert_eq!(
            classify_url("https://cdn.example.com/master.m3u8?token=1"),
            Some(MediaKind::Hls)
        );
        assert_eq!(
            classify_url("https://cdn.example.com/manifest.mpd"),
            Some(MediaKind::Dash)
        );
        assert_eq!(
            classify_url("https://cdn.example.com/video.mp4#t=10"),
            Some(MediaKind::Progressive)
        );
        assert_eq!(classify_url("https://example.com/page.html"), None);
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(
            classify_url("HTTPS://CDN.EXAMPLE.COM/MASTER.M3U8"),
            Some(MediaKind::Hls)
        );
    }
}
