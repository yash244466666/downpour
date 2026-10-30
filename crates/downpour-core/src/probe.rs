//! Probe: learn size, range support, validators, and file name before
//! transferring.
//!
//! `HEAD` is deliberately not used: many servers answer it incorrectly, and a
//! `Range: bytes=0-0` GET proves range support with the actual response. The
//! probe trusts a real `206` plus a valid `Content-Range`, never
//! `Accept-Ranges` alone.

use reqwest::header::{
    ACCEPT_ENCODING, AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_ENCODING, CONTENT_LENGTH,
    CONTENT_RANGE, CONTENT_TYPE, COOKIE, ETAG, HeaderMap, HeaderName, HeaderValue, LAST_MODIFIED,
    LOCATION, PROXY_AUTHORIZATION, RANGE,
};
use reqwest::{Client, StatusCode, Url};

use crate::error::{EngineError, EngineResult};

/// Redirect hops allowed before giving up.
pub const MAX_REDIRECTS: usize = 10;

/// What the probe learned about a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub final_url: String,
    pub total_bytes: Option<u64>,
    pub ranges_supported: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
    pub content_encoding: Option<String>,
}

impl ProbeResult {
    /// Strong validator usable in `If-Range`, preferring the ETag.
    #[must_use]
    pub fn validator(&self) -> Option<String> {
        match &self.etag {
            Some(etag) if !etag.starts_with("W/") => Some(etag.clone()),
            _ => self.last_modified.clone(),
        }
    }
}

/// Probe `url`, following redirects manually so credentials never cross an
/// origin boundary.
pub async fn probe(
    client: &Client,
    url: &Url,
    extra_headers: &HeaderMap,
) -> EngineResult<ProbeResult> {
    let mut current = url.clone();
    let mut headers = extra_headers.clone();
    headers.insert(RANGE, HeaderValue::from_static("bytes=0-0"));
    // Compressed bytes are not file offsets.
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));

    for _ in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .headers(headers.clone())
            .send()
            .await?;

        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    EngineError::Probe("redirect without a Location header".to_owned())
                })?;
            let next = current
                .join(location)
                .map_err(|err| EngineError::InvalidUrl(err.to_string()))?;
            if !matches!(next.scheme(), "http" | "https") {
                return Err(EngineError::UnsupportedScheme(next.scheme().to_owned()));
            }
            if next.origin() != current.origin() {
                headers.remove(AUTHORIZATION);
                headers.remove(COOKIE);
                headers.remove(PROXY_AUTHORIZATION);
            }
            current = next;
            continue;
        }

        let etag = header_string(response.headers(), ETAG);
        let last_modified = header_string(response.headers(), LAST_MODIFIED);
        let content_type = header_string(response.headers(), CONTENT_TYPE);
        let content_disposition = header_string(response.headers(), CONTENT_DISPOSITION);
        let content_encoding = header_string(response.headers(), CONTENT_ENCODING);
        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());

        let (total_bytes, ranges_supported) = match status {
            StatusCode::PARTIAL_CONTENT => {
                let parsed = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_content_range);
                match parsed {
                    Some((0, _, Some(total))) if total > 0 => (Some(total), true),
                    // A 206 we cannot interpret is treated as an unverifiable
                    // single stream, never as resume support. `Content-Length`
                    // describes only the ranged body, so it is not the size.
                    _ => (None, false),
                }
            }
            StatusCode::NO_CONTENT => (Some(0), false),
            StatusCode::OK => {
                let encoded = content_encoding
                    .as_deref()
                    .is_some_and(|value| !value.eq_ignore_ascii_case("identity"));
                (if encoded { None } else { content_length }, false)
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let total = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_unsatisfied_range);
                match total {
                    Some(0) => (Some(0), true),
                    _ => {
                        return Err(EngineError::Probe(
                            "server rejected the probe range request".to_owned(),
                        ));
                    }
                }
            }
            StatusCode::UNAUTHORIZED => return Err(EngineError::AuthRequired),
            StatusCode::PROXY_AUTHENTICATION_REQUIRED => {
                return Err(EngineError::ProxyAuthRequired);
            }
            other => {
                return Err(EngineError::Http {
                    status: other.as_u16(),
                });
            }
        };

        return Ok(ProbeResult {
            final_url: response.url().to_string(),
            total_bytes,
            ranges_supported,
            etag,
            last_modified,
            content_type,
            content_disposition,
            content_encoding,
        });
    }

    Err(EngineError::Probe(format!(
        "more than {MAX_REDIRECTS} redirects"
    )))
}

fn header_string(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

/// Parse `bytes start-end/total` (total may be `*`).
#[must_use]
pub fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    if start > end {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        value => Some(value.parse().ok()?),
    };
    Some((start, end, total))
}

/// Parse an unsatisfied-range header: `bytes */total`.
#[must_use]
pub fn parse_unsatisfied_range(value: &str) -> Option<u64> {
    value.trim().strip_prefix("bytes */")?.trim().parse().ok()
}

/// Extract a file name from a `Content-Disposition` header.
///
/// Prefers the RFC 5987 `filename*` form, then the plain `filename` parameter.
/// The result is still hostile input: callers must sanitize it.
#[must_use]
pub fn filename_from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;

    for part in split_parameters(value) {
        let Some((key, raw_value)) = part.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => extended = decode_extended_value(raw_value.trim()),
            "filename" => plain = Some(unescape_quoted(raw_value.trim())),
            _ => {}
        }
    }

    extended.or(plain).filter(|name| !name.is_empty())
}

/// Split on `;` while respecting quoted strings.
fn split_parameters(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (index, ch) in value.char_indices() {
        match ch {
            '\\' if in_quotes && !escaped => {
                escaped = true;
                continue;
            }
            '"' if !escaped => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
        escaped = false;
    }
    parts.push(&value[start..]);
    parts
}

fn unescape_quoted(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Decode an RFC 5987 `charset'lang'percent-encoded` value.
#[must_use]
pub fn decode_extended_value(value: &str) -> Option<String> {
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?.trim().to_ascii_lowercase();
    let _language = parts.next()?;
    let encoded = parts.next()?;
    let bytes = percent_decode(encoded);

    match charset.as_str() {
        "" | "utf-8" | "utf8" => Some(String::from_utf8_lossy(&bytes).into_owned()),
        "iso-8859-1" | "latin1" => Some(bytes.iter().map(|&byte| byte as char).collect()),
        _ => None,
    }
}

/// Percent-decode into raw bytes. Invalid escapes are passed through.
#[must_use]
pub fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Derive a file name from the last URL path segment.
#[must_use]
pub fn filename_from_url(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    let decoded = String::from_utf8_lossy(&percent_decode(segment)).into_owned();
    (!decoded.is_empty()).then_some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_ranges() {
        assert_eq!(
            parse_content_range("bytes 0-0/1234"),
            Some((0, 0, Some(1234)))
        );
        assert_eq!(parse_content_range("bytes 10-19/*"), Some((10, 19, None)));
        assert_eq!(parse_content_range("bytes 10-9/20"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes 0-0/abc"), None);
    }

    #[test]
    fn parses_unsatisfied_ranges() {
        assert_eq!(parse_unsatisfied_range("bytes */0"), Some(0));
        assert_eq!(parse_unsatisfied_range("bytes */1234"), Some(1234));
        assert_eq!(parse_unsatisfied_range("bytes 0-0/1"), None);
    }

    #[test]
    fn reads_plain_disposition_filenames() {
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"report.pdf\""),
            Some("report.pdf".to_owned())
        );
        assert_eq!(
            filename_from_content_disposition("inline;filename=no-quotes.zip"),
            Some("no-quotes.zip".to_owned())
        );
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"a;b.txt\""),
            Some("a;b.txt".to_owned())
        );
    }

    #[test]
    fn extended_disposition_wins() {
        assert_eq!(
            filename_from_content_disposition(
                "attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve%20file.txt"
            ),
            Some("naïve file.txt".to_owned())
        );
    }

    #[test]
    fn hostile_disposition_names_pass_through_for_sanitizing() {
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"../../etc/passwd\""),
            Some("../../etc/passwd".to_owned())
        );
    }

    #[test]
    fn percent_decoding_is_byte_exact() {
        assert_eq!(percent_decode("a%20b%2Fc"), b"a b/c".to_vec());
        assert_eq!(percent_decode("100%"), b"100%".to_vec());
        assert_eq!(percent_decode("%zz"), b"%zz".to_vec());
    }

    #[test]
    fn derives_names_from_urls() {
        let url = Url::parse("http://example.com/dir/file%20name.zip?token=1").unwrap();
        assert_eq!(filename_from_url(&url), Some("file name.zip".to_owned()));

        let trailing = Url::parse("http://example.com/dir/").unwrap();
        assert_eq!(filename_from_url(&trailing), Some("dir".to_owned()));

        let root = Url::parse("http://example.com/").unwrap();
        assert_eq!(filename_from_url(&root), None);
    }
}
