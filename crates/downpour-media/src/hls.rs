//! HLS parsing and a native fallback downloader.
//!
//! DASH is delegated to yt-dlp/ffmpeg and is not muxed here. HLS with
//! SAMPLE-AES (or any unknown key method) is refused: no DRM circumvention.

use std::path::Path;
use std::time::Duration;

use aes::Aes128;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use m3u8_rs::{KeyMethod, Playlist};
use reqwest::Client;
use serde::Serialize;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

type Aes128CbcDec = cbc::Decryptor<Aes128>;

#[derive(Debug, Error)]
pub enum HlsError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("playlist error: {0}")]
    Playlist(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("DRM-protected streams are not supported")]
    DrmUnsupported,
    #[error("playlist has no segments")]
    Empty,
    #[error("download canceled")]
    Canceled,
}

/// One quality level of a master playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    pub bandwidth: u64,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub uri: String,
}

/// Parse a master playlist and resolve its variant URLs.
pub fn parse_variants(base: &Url, body: &[u8]) -> Result<Vec<Variant>, HlsError> {
    match m3u8_rs::parse_playlist_res(body)
        .map_err(|error| HlsError::Playlist(error.to_string()))?
    {
        Playlist::MasterPlaylist(master) => Ok(master
            .variants
            .iter()
            .filter(|variant| !variant.is_i_frame)
            .filter_map(|variant| {
                let uri = base.join(&variant.uri).ok()?;
                Some(Variant {
                    bandwidth: variant.bandwidth,
                    width: variant.resolution.map(|resolution| resolution.width),
                    height: variant.resolution.map(|resolution| resolution.height),
                    uri: uri.to_string(),
                })
            })
            .collect()),
        Playlist::MediaPlaylist(_) => {
            Err(HlsError::Playlist("expected a master playlist".to_owned()))
        }
    }
}

/// Highest-bandwidth variant.
#[must_use]
pub fn best_variant(variants: &[Variant]) -> Option<&Variant> {
    variants.iter().max_by_key(|variant| variant.bandwidth)
}

#[derive(Debug, Clone)]
struct KeyInfo {
    uri: Url,
    iv: Option<[u8; 16]>,
}

#[derive(Debug, Clone)]
struct Segment {
    uri: Url,
    key: Option<KeyInfo>,
    sequence: u64,
}

#[derive(Debug)]
struct MediaPlan {
    init: Option<Url>,
    segments: Vec<Segment>,
}

fn parse_media_plan(base: &Url, body: &[u8]) -> Result<MediaPlan, HlsError> {
    let playlist = match m3u8_rs::parse_playlist_res(body)
        .map_err(|error| HlsError::Playlist(error.to_string()))?
    {
        Playlist::MediaPlaylist(playlist) => playlist,
        Playlist::MasterPlaylist(_) => {
            return Err(HlsError::Playlist("expected a media playlist".to_owned()));
        }
    };

    if playlist.segments.is_empty() {
        return Err(HlsError::Empty);
    }

    let mut init = None;
    let mut segments = Vec::with_capacity(playlist.segments.len());
    for (index, segment) in playlist.segments.iter().enumerate() {
        let key = match &segment.key {
            Some(key) => match key.method {
                KeyMethod::None => None,
                KeyMethod::AES128 => {
                    let uri = key
                        .uri
                        .as_deref()
                        .ok_or_else(|| HlsError::Playlist("AES-128 key without URI".to_owned()))?;
                    Some(KeyInfo {
                        uri: base
                            .join(uri)
                            .map_err(|error| HlsError::Playlist(error.to_string()))?,
                        iv: key.iv.as_deref().and_then(parse_iv),
                    })
                }
                KeyMethod::SampleAES | KeyMethod::Other(_) => return Err(HlsError::DrmUnsupported),
            },
            None => None,
        };

        if let Some(map) = &segment.map {
            let map_uri = base
                .join(&map.uri)
                .map_err(|error| HlsError::Playlist(error.to_string()))?;
            match &init {
                Some(existing) if existing != &map_uri => {
                    return Err(HlsError::Playlist(
                        "changing EXT-X-MAP is not supported".to_owned(),
                    ));
                }
                _ => init = Some(map_uri),
            }
        }

        segments.push(Segment {
            uri: base
                .join(&segment.uri)
                .map_err(|error| HlsError::Playlist(error.to_string()))?,
            key,
            sequence: playlist.media_sequence + index as u64,
        });
    }

    Ok(MediaPlan { init, segments })
}

fn parse_iv(value: &str) -> Option<[u8; 16]> {
    let value = value
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    let bytes = hex::decode(value).ok()?;
    let mut iv = [0_u8; 16];
    if bytes.len() > 16 {
        return None;
    }
    iv[16 - bytes.len()..].copy_from_slice(&bytes);
    Some(iv)
}

fn sequence_iv(sequence: u64) -> [u8; 16] {
    let mut iv = [0_u8; 16];
    iv[8..].copy_from_slice(&sequence.to_be_bytes());
    iv
}

fn decrypt_aes128(data: &[u8], key: &[u8], iv: &[u8; 16]) -> Result<Vec<u8>, HlsError> {
    let mut buffer = data.to_vec();
    let plaintext = Aes128CbcDec::new_from_slices(key, iv)
        .map_err(|error| HlsError::Playlist(error.to_string()))?
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|_| HlsError::Playlist("AES-128 padding is invalid".to_owned()))?;
    Ok(plaintext.to_vec())
}

/// Download an HLS stream (master or media playlist) into `destination`.
///
/// `cancel` is checked between segments. Progress reports the number of bytes
/// written and the total segment count.
pub async fn download_hls(
    client: &Client,
    playlist_url: &Url,
    destination: &Path,
    cancel: &(dyn Fn() -> bool + Sync),
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<u64, HlsError> {
    let body = fetch_playlist(client, playlist_url).await?;
    let parsed = m3u8_rs::parse_playlist_res(&body)
        .map_err(|error| HlsError::Playlist(error.to_string()))?;

    let media_url = match parsed {
        Playlist::MasterPlaylist(_) => {
            let variants = parse_variants(playlist_url, &body)?;
            let best = best_variant(&variants).ok_or(HlsError::Empty)?;
            Url::parse(&best.uri).map_err(|error| HlsError::Playlist(error.to_string()))?
        }
        Playlist::MediaPlaylist(_) => playlist_url.clone(),
    };

    let media_body = if media_url == *playlist_url {
        body
    } else {
        fetch_playlist(client, &media_url).await?
    };
    let plan = parse_media_plan(&media_url, &media_body)?;

    let mut file = tokio::fs::File::create(destination).await?;
    let mut written = 0_u64;

    if let Some(init) = plan.init {
        let bytes = fetch_bytes(client, &init).await?;
        file.write_all(&bytes).await?;
        written += bytes.len() as u64;
    }

    let total_segments = plan.segments.len() as u64;
    let mut key_cache: std::collections::HashMap<String, Vec<u8>> =
        std::collections::HashMap::new();

    for (index, segment) in plan.segments.iter().enumerate() {
        if cancel() {
            return Err(HlsError::Canceled);
        }
        let mut bytes = fetch_with_retry(client, &segment.uri).await?;
        if let Some(key) = &segment.key {
            let cache_key = key.uri.to_string();
            let key_bytes = if let Some(cached) = key_cache.get(&cache_key) {
                cached.clone()
            } else {
                let fetched = fetch_with_retry(client, &key.uri).await?;
                key_cache.insert(cache_key, fetched.clone());
                fetched
            };
            let iv = key.iv.unwrap_or_else(|| sequence_iv(segment.sequence));
            bytes = decrypt_aes128(&bytes, &key_bytes, &iv)?;
        }
        file.write_all(&bytes).await?;
        written += bytes.len() as u64;
        progress(written, Some(total_segments));
        let _ = index;
    }

    file.flush().await?;
    Ok(written)
}

async fn fetch_playlist(client: &Client, url: &Url) -> Result<Vec<u8>, HlsError> {
    let response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

async fn fetch_bytes(client: &Client, url: &Url) -> Result<Vec<u8>, HlsError> {
    let response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

async fn fetch_with_retry(client: &Client, url: &Url) -> Result<Vec<u8>, HlsError> {
    let mut last_error = None;
    for attempt in 0..3 {
        match fetch_bytes(client, url).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
            }
        }
    }
    Err(last_error.unwrap_or(HlsError::Empty))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};

    const MASTER: &[u8] = b"#EXTM3U\n\
        #EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360\n\
        low.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=1280x720\n\
        high.m3u8\n";

    const MEDIA: &[u8] = b"#EXTM3U\n\
        #EXT-X-TARGETDURATION:4\n\
        #EXT-X-MEDIA-SEQUENCE:5\n\
        #EXTINF:4,\n\
        seg0.ts\n\
        #EXTINF:4,\n\
        seg1.ts\n";

    #[test]
    fn picks_the_highest_bandwidth() {
        let base = Url::parse("https://cdn.example.com/master.m3u8").unwrap();
        let variants = parse_variants(&base, MASTER).unwrap();
        assert_eq!(variants.len(), 2);
        let best = best_variant(&variants).unwrap();
        assert_eq!(best.bandwidth, 900_000);
        assert_eq!(best.uri, "https://cdn.example.com/high.m3u8");
        assert_eq!(best.height, Some(720));
    }

    #[test]
    fn resolves_relative_segments() {
        let base = Url::parse("https://cdn.example.com/media.m3u8").unwrap();
        let plan = parse_media_plan(&base, MEDIA).unwrap();
        assert_eq!(plan.segments.len(), 2);
        assert_eq!(
            plan.segments[0].uri.as_str(),
            "https://cdn.example.com/seg0.ts"
        );
        assert_eq!(plan.segments[0].sequence, 5);
    }

    #[test]
    fn refuses_sample_aes() {
        let body = b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
            #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key.bin\"\n\
            #EXTINF:4,\nseg.ts\n";
        let base = Url::parse("https://cdn.example.com/media.m3u8").unwrap();
        assert!(matches!(
            parse_media_plan(&base, body),
            Err(HlsError::DrmUnsupported)
        ));
    }

    #[test]
    fn aes_round_trip() {
        type Aes128CbcEnc = cbc::Encryptor<Aes128>;
        let key = [7_u8; 16];
        let iv = [3_u8; 16];
        let plaintext = b"downpour hls payload";
        let mut buffer = vec![0_u8; plaintext.len() + 16];
        buffer[..plaintext.len()].copy_from_slice(plaintext);
        let ciphertext = Aes128CbcEnc::new(&key.into(), &iv.into())
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
            .unwrap()
            .to_vec();
        let decrypted = decrypt_aes128(&ciphertext, &key, &iv).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn parses_ivs() {
        assert_eq!(
            parse_iv("0x00000000000000000000000000000002"),
            Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2])
        );
        assert_eq!(parse_iv("zz"), None);
    }
}
