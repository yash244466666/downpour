//! Batch URL expansion and (later) the robots-aware site crawler.

/// Hard cap so a malicious pattern cannot allocate unbounded work.
pub const MAX_EXPANSIONS: usize = 10_000;

/// Expand IDM-style batch patterns:
///
/// - `img[001-003].jpg` → `img001.jpg`, `img002.jpg`, `img003.jpg`
/// - `file{1,2}.txt` → `file1.txt`, `file2.txt`
/// - patterns nest: `[1-2]/{a,b}.png` → four URLs
///
/// A malformed or reversed range is left literal. Expansion is capped at
/// [`MAX_EXPANSIONS`].
#[must_use]
pub fn expand_patterns(input: &str) -> Vec<String> {
    let mut results = vec![input.to_owned()];

    // Each pass expands the first pattern in every string. Bounded nesting.
    for _ in 0..32 {
        let mut changed = false;
        let mut next = Vec::with_capacity(results.len());

        for pattern in results {
            match expand_once(&pattern) {
                Some(expanded) if !expanded.is_empty() => {
                    changed = true;
                    next.extend(expanded);
                }
                _ => next.push(pattern),
            }
        }

        results = next;
        if !changed {
            break;
        }
        if results.len() >= MAX_EXPANSIONS {
            results.truncate(MAX_EXPANSIONS);
            break;
        }
    }

    results
}

fn expand_once(pattern: &str) -> Option<Vec<String>> {
    if let Some((open, close)) = find_numeric_range(pattern) {
        let (start, end, width) = parse_range(&pattern[open + 1..close])?;
        let prefix = &pattern[..open];
        let suffix = &pattern[close + 1..];
        let mut out = Vec::with_capacity((end - start + 1) as usize);

        for n in start..=end {
            let value = if width > 0 {
                format!("{n:0width$}")
            } else {
                n.to_string()
            };
            out.push(format!("{prefix}{value}{suffix}"));
        }
        return Some(out);
    }

    let open = pattern.find('{')?;
    let close = open + 1 + pattern[open + 1..].find('}')?;
    let inner = &pattern[open + 1..close];
    let parts: Vec<&str> = inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 2 {
        return None;
    }

    let prefix = &pattern[..open];
    let suffix = &pattern[close + 1..];
    Some(
        parts
            .into_iter()
            .map(|part| format!("{prefix}{part}{suffix}"))
            .collect(),
    )
}

fn find_numeric_range(pattern: &str) -> Option<(usize, usize)> {
    for (open, ch) in pattern.char_indices() {
        if ch != '[' {
            continue;
        }
        let Some(relative) = pattern[open + 1..].find(']') else {
            continue;
        };
        let close = open + 1 + relative;
        if parse_range(&pattern[open + 1..close]).is_some() {
            return Some((open, close));
        }
    }
    None
}

/// Parse `start-end`. Returns `(start, end, zero_pad_width)`; width 0 means no
/// padding was requested.
fn parse_range(inner: &str) -> Option<(u64, u64, usize)> {
    let (left, right) = inner.split_once('-')?;
    let left = left.trim();
    let right = right.trim();

    if left.is_empty()
        || right.is_empty()
        || !left.bytes().all(|b| b.is_ascii_digit())
        || !right.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let start: u64 = left.parse().ok()?;
    let end: u64 = right.parse().ok()?;
    if start > end || end - start + 1 > MAX_EXPANSIONS as u64 {
        return None;
    }

    let padded =
        (left.len() > 1 && left.starts_with('0')) || (right.len() > 1 && right.starts_with('0'));
    let width = if padded {
        left.len().max(right.len())
    } else {
        0
    };

    Some((start, end, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_zero_padded_range() {
        assert_eq!(
            expand_patterns("img[001-003].jpg"),
            vec!["img001.jpg", "img002.jpg", "img003.jpg"]
        );
    }

    #[test]
    fn expands_unpadded_range() {
        assert_eq!(
            expand_patterns("file[1-3].txt"),
            vec!["file1.txt", "file2.txt", "file3.txt"]
        );
    }

    #[test]
    fn expands_brace_alternatives() {
        assert_eq!(
            expand_patterns("file{one,two}.zip"),
            vec!["fileone.zip", "filetwo.zip"]
        );
    }

    #[test]
    fn expands_nested_patterns() {
        assert_eq!(
            expand_patterns("img[1-2]/{a,b}.png"),
            vec!["img1/a.png", "img1/b.png", "img2/a.png", "img2/b.png"]
        );
    }

    #[test]
    fn leaves_plain_and_malformed_patterns_alone() {
        assert_eq!(expand_patterns("plain.txt"), vec!["plain.txt"]);
        assert_eq!(expand_patterns("img[3-1].jpg"), vec!["img[3-1].jpg"]);
        assert_eq!(expand_patterns("img[abc].jpg"), vec!["img[abc].jpg"]);
        assert_eq!(expand_patterns("only{one}.txt"), vec!["only{one}.txt"]);
    }

    #[test]
    fn caps_expansion_count() {
        let out = expand_patterns("x[1-99999].bin");
        // Range exceeds the cap, so it stays literal.
        assert_eq!(out, vec!["x[1-99999].bin"]);
    }
}
