//! Cross-platform file-name resolution and sanitization.
//!
//! Download names come from untrusted servers, so every path component is
//! sanitized before it touches the filesystem.

/// Conservative upper bound for a single path component (most filesystems
/// allow 255 bytes; leave room for collision suffixes).
const MAX_NAME_BYTES: usize = 200;

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Sanitize a server-provided name into a safe single path component.
///
/// - strips any directory components (`/`, `\`, and Windows drive prefixes)
/// - replaces characters illegal on Windows and path separators
/// - drops control characters
/// - avoids Windows reserved device names
/// - trims trailing dots/spaces (Windows)
/// - caps the byte length while preserving the extension
/// - falls back to `download` for empty or dot-only names
#[must_use]
pub fn sanitize_file_name(input: &str) -> String {
    let base = input
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(input)
        .trim_start_matches(|c: char| c == ':' || c.is_whitespace());

    let mut cleaned = String::with_capacity(base.len());
    for ch in base.chars() {
        match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => cleaned.push('_'),
            c if c.is_control() => {}
            c => cleaned.push(c),
        }
    }

    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    let mut name = trimmed.to_owned();

    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }

    if name.is_empty() || name == "." || name == ".." {
        name = "download".to_owned();
    }

    truncate_preserving_extension(&name, MAX_NAME_BYTES)
}

/// Truncate a name to `max_bytes` without splitting a UTF-8 character or the
/// extension. The extension itself is capped so the result always fits.
#[must_use]
pub fn truncate_preserving_extension(name: &str, max_bytes: usize) -> String {
    if name.len() <= max_bytes {
        return name.to_owned();
    }

    let (stem, ext) = split_extension(name);
    let ext = if ext.len() + 1 >= max_bytes { "" } else { ext };
    let budget = max_bytes.saturating_sub(ext.len() + usize::from(!ext.is_empty()));
    let stem = truncate_utf8(stem, budget);

    if ext.is_empty() {
        stem.to_owned()
    } else {
        format!("{stem}.{ext}")
    }
}

/// Split `name.ext` at the last dot. Dotfiles are treated as stem-only.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(idx) if idx > 0 && idx + 1 < name.len() => (&name[..idx], &name[idx + 1..]),
        _ => (name, ""),
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Append ` (n)` before the extension until the path does not exist.
#[must_use]
pub fn unique_file_name(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let (stem, ext) = split_extension(name);
    for n in 1..=9999_u32 {
        let candidate = if ext.is_empty() {
            dir.join(format!("{stem} ({n})"))
        } else {
            dir.join(format!("{stem} ({n}).{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    // Extremely unlikely: fall back to a UUID-ish suffix.
    dir.join(format!("{stem}-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_path_components() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_file_name(r"C:\Windows\system32\evil.exe"),
            "evil.exe"
        );
        assert_eq!(sanitize_file_name("/tmp/report.pdf"), "report.pdf");
    }

    #[test]
    fn replaces_illegal_characters() {
        assert_eq!(sanitize_file_name("a<b>c:d\"e|f?g*h"), "a_b_c_d_e_f_g_h");
        assert_eq!(sanitize_file_name("tab\there"), "tabhere");
    }

    #[test]
    fn handles_reserved_windows_names() {
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("con.txt"), "_con.txt");
        assert_eq!(sanitize_file_name("LPT9.log"), "_LPT9.log");
        assert_eq!(sanitize_file_name("console.txt"), "console.txt");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_file_name("report.pdf... "), "report.pdf");
    }

    #[test]
    fn empty_and_dot_names_fall_back() {
        assert_eq!(sanitize_file_name(""), "download");
        assert_eq!(sanitize_file_name("..."), "download");
        assert_eq!(sanitize_file_name("   "), "download");
    }

    #[test]
    fn truncation_preserves_extension_and_char_boundaries() {
        let name = format!("{}.pdf", "é".repeat(300));
        let out = sanitize_file_name(&name);
        assert!(out.len() <= MAX_NAME_BYTES);
        assert!(out.ends_with(".pdf"));
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn unique_name_appends_counter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"x").unwrap();
        let path = unique_file_name(dir.path(), "file.txt");
        assert_eq!(path.file_name().unwrap(), "file (1).txt");
    }
}
