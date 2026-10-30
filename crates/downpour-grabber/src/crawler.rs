//! Robots-aware breadth-first site crawler.
//!
//! The crawler discovers file URLs without fetching them: pages are fetched,
//! links are extracted, file links are recorded, and page links are queued up
//! to the depth cap. Same-origin is the default; robots.txt is honoured.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use reqwest::Client;
use scraper::{Html, Selector};
use serde::Serialize;
use texting_robots::Robot;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum GrabError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("robots.txt error: {0}")]
    Robots(String),
    #[error("canceled")]
    Canceled,
}

/// Crawl limits and filters.
#[derive(Debug, Clone)]
pub struct GrabOptions {
    pub max_depth: u32,
    pub max_pages: u32,
    pub same_origin: bool,
    pub respect_robots: bool,
    pub delay: Duration,
    pub file_extensions: Vec<String>,
}

impl Default for GrabOptions {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_pages: 100,
            same_origin: true,
            respect_robots: true,
            delay: Duration::from_millis(500),
            file_extensions: default_extensions(),
        }
    }
}

fn default_extensions() -> Vec<String> {
    [
        "zip", "rar", "7z", "tar", "gz", "xz", "bz2", "zst", "iso", "img", "pdf", "epub", "doc",
        "docx", "xls", "xlsx", "ppt", "pptx", "txt", "csv", "mp3", "m4a", "flac", "wav", "ogg",
        "opus", "mp4", "mkv", "webm", "mov", "avi", "m4v", "ts", "jpg", "jpeg", "png", "gif",
        "webp", "svg", "avif", "exe", "msi", "dmg", "appimage", "deb", "rpm", "apk",
    ]
    .iter()
    .map(|extension| (*extension).to_owned())
    .collect()
}

/// One successfully visited page.
#[derive(Debug, Clone, Serialize)]
pub struct PageVisit {
    pub url: String,
    pub depth: u32,
    pub links: u32,
    pub files: u32,
}

/// One discovered file URL.
#[derive(Debug, Clone, Serialize)]
pub struct FoundFile {
    pub url: String,
    pub depth: u32,
    pub source_page: String,
}

#[derive(Debug, Default, Serialize)]
pub struct GrabResult {
    pub pages: Vec<PageVisit>,
    pub files: Vec<FoundFile>,
}

/// Persistence hook for a crawler frontier.
pub trait Frontier {
    fn discovered(&mut self, url: &str, depth: u32);
    fn visited(&mut self, url: &str);
}

impl Frontier for () {
    fn discovered(&mut self, _url: &str, _depth: u32) {}
    fn visited(&mut self, _url: &str) {}
}

/// Crawl `seed`, returning visited pages and discovered files.
pub async fn crawl(
    client: &Client,
    seed: Url,
    options: &GrabOptions,
    frontier: &mut dyn Frontier,
    cancel: &(dyn Fn() -> bool + Sync),
    mut progress: impl FnMut(&PageVisit),
) -> Result<GrabResult, GrabError> {
    let seed_origin = seed.origin().ascii_serialization();
    let robots = if options.respect_robots {
        fetch_robots(client, &seed).await?
    } else {
        None
    };

    let mut queue: VecDeque<(Url, u32)> = VecDeque::from([(seed.clone(), 0)]);
    let mut queued: HashSet<String> = HashSet::from([canonical_url(&seed)]);
    let mut files: HashSet<String> = HashSet::new();
    let mut result = GrabResult::default();
    let mut last_request: Option<Instant> = None;

    while let Some((url, depth)) = queue.pop_front() {
        if cancel() {
            return Err(GrabError::Canceled);
        }
        if result.pages.len() as u32 >= options.max_pages {
            break;
        }
        if options.same_origin && url.origin().ascii_serialization() != seed_origin {
            continue;
        }
        if let Some(robot) = &robots {
            if !robot.allowed(url.as_str()) {
                continue;
            }
        }

        if let Some(last) = last_request {
            let elapsed = last.elapsed();
            if elapsed < options.delay {
                tokio::time::sleep(options.delay - elapsed).await;
            }
        }

        frontier.discovered(url.as_str(), depth);
        let response = match client.get(url.clone()).send().await {
            Ok(response) => response,
            Err(_) => {
                last_request = Some(Instant::now());
                continue;
            }
        };
        last_request = Some(Instant::now());
        if !response.status().is_success() {
            continue;
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
            // A directly linked non-page resource is itself the target.
            push_file(&mut result, &mut files, &url, depth, &url);
            continue;
        }

        let body = match response.text().await {
            Ok(body) => body,
            Err(_) => continue,
        };
        let links = extract_links(&body, &url);
        let mut visit = PageVisit {
            url: url.to_string(),
            depth,
            links: links.len() as u32,
            files: 0,
        };

        for link in links {
            if is_file_url(&link, &options.file_extensions) {
                if let Some(robot) = &robots {
                    if !robot.allowed(link.as_str()) {
                        continue;
                    }
                }
                if push_file(&mut result, &mut files, &link, depth, &url) {
                    visit.files += 1;
                }
                continue;
            }
            if depth >= options.max_depth {
                continue;
            }
            let key = canonical_url(&link);
            if queued.insert(key) {
                queue.push_back((link, depth + 1));
            }
        }

        frontier.visited(url.as_str());
        progress(&visit);
        result.pages.push(visit);
    }

    result.files.sort_by(|a, b| a.url.cmp(&b.url));
    Ok(result)
}

fn push_file(
    result: &mut GrabResult,
    seen: &mut HashSet<String>,
    url: &Url,
    depth: u32,
    source: &Url,
) -> bool {
    let key = canonical_url(url);
    if !seen.insert(key) {
        return false;
    }
    result.files.push(FoundFile {
        url: url.to_string(),
        depth,
        source_page: source.to_string(),
    });
    true
}

async fn fetch_robots(client: &Client, seed: &Url) -> Result<Option<Robot>, GrabError> {
    let mut robots_url = seed.clone();
    robots_url.set_path("/robots.txt");
    robots_url.set_query(None);
    robots_url.set_fragment(None);

    let response = match client.get(robots_url).send().await {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    let body = response.text().await?;
    Robot::new("Downpour", body.as_bytes())
        .map(Some)
        .map_err(|error| GrabError::Robots(error.to_string()))
}

/// Extract absolute http(s) links from HTML, including srcset and CSS `url()`.
#[must_use]
pub fn extract_links(html: &str, base: &Url) -> Vec<Url> {
    let document = Html::parse_document(html);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let attributes: [(&str, &str); 9] = [
        ("a[href]", "href"),
        ("link[href]", "href"),
        ("img[src]", "src"),
        ("script[src]", "src"),
        ("video[src]", "src"),
        ("audio[src]", "src"),
        ("source[src]", "src"),
        ("iframe[src]", "src"),
        ("form[action]", "action"),
    ];
    for (selector, attribute) in attributes {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        for element in document.select(&selector) {
            if let Some(value) = element.value().attr(attribute) {
                push_link(&mut out, &mut seen, base, value);
            }
        }
    }

    if let Ok(selector) = Selector::parse("[srcset]") {
        for element in document.select(&selector) {
            if let Some(value) = element.value().attr("srcset") {
                for candidate in value.split(',') {
                    if let Some(url) = candidate.split_whitespace().next() {
                        push_link(&mut out, &mut seen, base, url);
                    }
                }
            }
        }
    }

    if let Ok(selector) = Selector::parse("[style]") {
        for element in document.select(&selector) {
            if let Some(value) = element.value().attr("style") {
                for raw in css_urls(value) {
                    push_link(&mut out, &mut seen, base, &raw);
                }
            }
        }
    }
    if let Ok(selector) = Selector::parse("style") {
        for element in document.select(&selector) {
            for raw in css_urls(&element.text().collect::<String>()) {
                push_link(&mut out, &mut seen, base, &raw);
            }
        }
    }

    out
}

fn push_link(out: &mut Vec<Url>, seen: &mut HashSet<String>, base: &Url, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.starts_with('#')
        || raw.starts_with("mailto:")
        || raw.starts_with("javascript:")
        || raw.starts_with("data:")
        || raw.starts_with("tel:")
    {
        return;
    }
    let Ok(mut url) = base.join(raw) else {
        return;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }
    url.set_fragment(None);
    if seen.insert(canonical_url(&url)) {
        out.push(url);
    }
}

fn css_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("url(") {
        let after = &rest[index + 4..];
        let Some(end) = after.find(')') else {
            break;
        };
        let raw = after[..end].trim().trim_matches(['"', '\'']);
        if !raw.is_empty() {
            out.push(raw.to_owned());
        }
        rest = &after[end + 1..];
    }
    out
}

/// Canonical dedupe key: no fragment, lowercase host, no default port.
#[must_use]
pub fn canonical_url(url: &Url) -> String {
    let mut clone = url.clone();
    clone.set_fragment(None);
    clone.to_string()
}

/// Does the URL's path end in a known file extension?
#[must_use]
pub fn is_file_url(url: &Url, extensions: &[String]) -> bool {
    let path = url.path().to_ascii_lowercase();
    let Some(extension) = path.rsplit_once('.').map(|(_, extension)| extension) else {
        return false;
    };
    if extension.len() > 10 || extension.contains('/') {
        return false;
    }
    extensions.iter().any(|candidate| candidate == extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/docs/page.html").unwrap()
    }

    #[test]
    fn extracts_links_from_common_elements() {
        let html = r##"
            <html><head>
              <link rel="stylesheet" href="/style.css">
              <style>.hero { background: url('/img/hero.jpg'); }</style>
            </head><body>
              <a href="next.html">next</a>
              <a href="mailto:a@example.com">mail</a>
              <a href="#fragment">skip</a>
              <img src="/img/a.png" srcset="/img/a@2x.png 2x, /img/a@3x.png 3x">
              <video src="/media/clip.mp4"><source src="/media/clip.webm"></video>
              <div style="background-image: url(/img/bg.png)"></div>
            </body></html>"##;
        let links = extract_links(html, &base());
        let strings: Vec<String> = links.iter().map(ToString::to_string).collect();
        assert!(strings.contains(&"https://example.com/docs/next.html".to_owned()));
        assert!(strings.contains(&"https://example.com/img/a.png".to_owned()));
        assert!(strings.contains(&"https://example.com/img/a@2x.png".to_owned()));
        assert!(strings.contains(&"https://example.com/media/clip.mp4".to_owned()));
        assert!(strings.contains(&"https://example.com/media/clip.webm".to_owned()));
        assert!(strings.contains(&"https://example.com/img/hero.jpg".to_owned()));
        assert!(strings.contains(&"https://example.com/img/bg.png".to_owned()));
        assert!(!strings.iter().any(|link| link.contains("mailto")));
        assert!(!strings.iter().any(|link| link.contains('#')));
    }

    #[test]
    fn classifies_files_by_extension() {
        let extensions = default_extensions();
        let file = Url::parse("https://example.com/a/b/FILE.ZIP?x=1").unwrap();
        let page = Url::parse("https://example.com/a/b/index.html").unwrap();
        let no_extension = Url::parse("https://example.com/download").unwrap();
        assert!(is_file_url(&file, &extensions));
        assert!(!is_file_url(&page, &extensions));
        assert!(!is_file_url(&no_extension, &extensions));
    }

    #[test]
    fn canonical_drops_fragments() {
        let url = Url::parse("https://example.com/a#section").unwrap();
        assert_eq!(canonical_url(&url), "https://example.com/a");
    }
}
