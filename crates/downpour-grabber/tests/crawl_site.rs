//! Crawl a local fixture site: robots, depth, dedupe, and file discovery.

use std::time::Duration;

use downpour_grabber::{GrabOptions, crawl};
use downpour_testkit::TestServer;
use url::Url;

#[tokio::test]
async fn crawls_within_limits_and_honours_robots() {
    let server = TestServer::start().await.unwrap();
    let seed = Url::parse(&server.url("/site/index.html")).unwrap();
    let client = reqwest::Client::new();
    let options = GrabOptions {
        max_depth: 2,
        max_pages: 10,
        same_origin: true,
        respect_robots: true,
        delay: Duration::ZERO,
        ..GrabOptions::default()
    };

    let result = crawl(&client, seed, &options, &mut (), &|| false, |_visit| {})
        .await
        .expect("crawl");

    let pages: Vec<&str> = result.pages.iter().map(|page| page.url.as_str()).collect();
    assert!(pages.iter().any(|url| url.ends_with("/site/index.html")));
    assert!(pages.iter().any(|url| url.ends_with("/site/a.html")));
    assert!(pages.iter().any(|url| url.ends_with("/site/b.html")));
    assert!(!pages.iter().any(|url| url.contains("example.invalid")));

    let files: Vec<&str> = result.files.iter().map(|file| file.url.as_str()).collect();
    assert!(files.iter().any(|url| url.ends_with("file1.zip")));
    assert!(files.iter().any(|url| url.ends_with("file2.zip")));
    assert!(files.iter().any(|url| url.ends_with("img/pic.png")));
    assert!(
        !files.iter().any(|url| url.contains("private/secret.zip")),
        "robots.txt must filter files too"
    );

    server.stop().await;
}

#[tokio::test]
async fn page_cap_is_enforced() {
    let server = TestServer::start().await.unwrap();
    let seed = Url::parse(&server.url("/site/index.html")).unwrap();
    let client = reqwest::Client::new();
    let options = GrabOptions {
        max_depth: 5,
        max_pages: 1,
        same_origin: true,
        respect_robots: false,
        delay: Duration::ZERO,
        ..GrabOptions::default()
    };

    let result = crawl(&client, seed, &options, &mut (), &|| false, |_| {})
        .await
        .expect("crawl");
    assert_eq!(result.pages.len(), 1);
    server.stop().await;
}
