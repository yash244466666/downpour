//! Native HLS fallback against the local fixture server.

use downpour_media::hls::download_hls;
use downpour_testkit::{TestServer, pattern_byte};

#[tokio::test]
async fn downloads_the_best_variant() {
    let server = TestServer::start().await.unwrap();
    let url = url::Url::parse(&server.url("/hls/master.m3u8")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("stream.ts");
    let client = reqwest::Client::new();

    let mut samples = Vec::new();
    let written = download_hls(&client, &url, &destination, &|| false, |bytes, total| {
        samples.push((bytes, total));
    })
    .await
    .expect("hls download");

    assert_eq!(written, 3 * 1024);
    let contents = std::fs::read(&destination).unwrap();
    let expected: Vec<u8> = (0..3 * 1024)
        .map(|offset| pattern_byte(offset as u64))
        .collect();
    assert_eq!(contents, expected);
    assert!(samples.len() >= 2, "expected per-segment progress");

    server.stop().await;
}
