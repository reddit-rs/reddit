//! Parallel media downloader with caching, retries and URL fallbacks.

use crate::clean::ext_from_url;
use crate::models::ManifestItem;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use wreq::Client;
use wreq::header::{HeaderMap, HeaderValue};
use wreq_util::Emulation;

/// Path of a manifest item relative to the `media/` directory
/// (e.g. `posts/abc123_00.jpg`).
fn file_name(item: &ManifestItem) -> PathBuf {
    let ext = item
        .ext
        .clone()
        .or_else(|| ext_from_url(&item.url))
        .unwrap_or_else(|| "jpg".to_string());
    let name = match item.kind.as_str() {
        "icon" => format!("subreddit_icon.{ext}"),
        "banner" => format!("subreddit_banner.{ext}"),
        "video" => match item.index {
            Some(i) => format!("{}_{i:02}.mp4", item.id),
            None => format!("{}.mp4", item.id),
        },
        "thumb" => format!("{}_thumb.{ext}", item.id),
        "cover" => format!("{}_cover.{ext}", item.id),
        _ => match item.index {
            Some(i) => format!("{}_{i:02}.{ext}", item.id),
            None => format!("{}.{ext}", item.id),
        },
    };
    PathBuf::from(&item.folder).join(name)
}

/// On-disk path of a manifest item relative to the output directory
/// (e.g. `media/posts/abc123_00.jpg`). Used by the offline viewer.
pub(crate) fn manifest_rel_path(item: &ManifestItem) -> PathBuf {
    PathBuf::from("media").join(file_name(item))
}

fn media_headers(ua: &str, cookie_header: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if !ua.is_empty() {
        headers.insert("User-Agent", HeaderValue::from_str(ua)?);
    }
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://www.reddit.com/"),
    );
    if let Some(c) = cookie_header
        && let Ok(v) = HeaderValue::from_str(c)
    {
        headers.insert("Cookie", v);
    }
    Ok(headers)
}

/// Download every manifest item into `<outdir>/media`, skipping files that
/// already exist. Returns `(downloaded, failed)`.
pub async fn download_all(
    manifest: &[ManifestItem],
    outdir: &Path,
    ua: &str,
    cookie_header: Option<&str>,
    concurrency: usize,
) -> (usize, usize) {
    let client = Client::builder()
        .emulation(Emulation::Chrome131)
        .timeout(Duration::from_secs(120))
        .build()
        .expect("failed to build download client");
    let headers = match media_headers(ua, cookie_header) {
        Ok(h) => h,
        Err(e) => {
            println!("media headers invalid: {e}");
            return (0, manifest.len());
        }
    };
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let meddir = outdir.join("media");
    let mut handles = Vec::with_capacity(manifest.len());

    for item in manifest {
        let client = client.clone();
        let sem = sem.clone();
        let meddir = meddir.clone();
        let headers = headers.clone();
        let item = item.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            download_one(&client, &item, &meddir, &headers).await
        }));
    }

    let total = manifest.len();
    let mut failed = 0usize;
    let mut done = 0usize;
    for h in handles {
        match h.await {
            Ok(Ok(msg)) => {
                done += 1;
                if msg.starts_with("FAIL") {
                    failed += 1;
                    println!("[{done}/{total}] {msg}");
                } else if done.is_multiple_of(25) {
                    println!("[{done}/{total}] {msg}");
                }
            }
            Ok(Err(e)) => {
                done += 1;
                failed += 1;
                println!("[{done}/{total}] FAIL {e}");
            }
            Err(e) => {
                done += 1;
                failed += 1;
                println!("[{done}/{total}] FAIL task error: {e}");
            }
        }
    }
    println!(
        "\nmedia: {}/{} downloaded, {failed} failed",
        total - failed,
        total
    );
    (total - failed, failed)
}

async fn download_one(
    client: &Client,
    item: &ManifestItem,
    meddir: &Path,
    headers: &HeaderMap,
) -> Result<String> {
    let rel = file_name(item);
    let path = meddir.join(&rel);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::metadata(&path)
        .await
        .map(|m| m.len() > 0)
        .unwrap_or(false)
    {
        return Ok(format!("cached  {}", rel.display()));
    }

    let mut urls = vec![item.url.clone()];
    if let Some(f) = &item.fallback
        && f != &item.url
    {
        urls.push(f.clone());
    }

    for url in urls {
        for attempt in 0..3 {
            let resp = match client.get(&url).headers(headers.clone()).send().await {
                Ok(r) => r,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if status == 200 {
                let bytes = match resp.bytes().await {
                    Ok(b) => b,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                        continue;
                    }
                };
                if !bytes.is_empty() {
                    let mut f = tokio::fs::File::create(&path).await?;
                    f.write_all(&bytes).await?;
                    f.flush().await?;
                    return Ok(format!("OK      {}", rel.display()));
                }
            } else if status == 429 {
                let wait = 15 * (attempt + 1);
                println!("  rate limited on media (429), waiting {wait}s...");
                tokio::time::sleep(Duration::from_secs(wait)).await;
            } else if status == 404 {
                break; // fall through to the next candidate URL
            } else {
                tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
            }
        }
    }
    Ok(format!("FAIL    {}", rel.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, id: &str, index: Option<usize>) -> ManifestItem {
        ManifestItem {
            folder: "posts".into(),
            id: id.into(),
            kind: kind.into(),
            url: "https://i.redd.it/x.jpg".into(),
            fallback: None,
            ext: Some("jpg".into()),
            index,
            width: None,
            height: None,
        }
    }

    #[test]
    fn file_names_cover_all_kinds() {
        assert_eq!(
            manifest_rel_path(&item("image", "abc", None)),
            PathBuf::from("media/posts/abc.jpg")
        );
        assert_eq!(
            manifest_rel_path(&item("gallery", "abc", Some(3))),
            PathBuf::from("media/posts/abc_03.jpg")
        );
        assert_eq!(
            manifest_rel_path(&item("thumb", "abc", None)),
            PathBuf::from("media/posts/abc_thumb.jpg")
        );
        assert_eq!(
            manifest_rel_path(&item("video", "abc", Some(1))),
            PathBuf::from("media/posts/abc_01.mp4")
        );
        let icon = ManifestItem {
            folder: String::new(),
            id: "subreddit".into(),
            kind: "icon".into(),
            url: "https://styles.redditmedia.com/i.png?w=256".into(),
            fallback: None,
            ext: None,
            index: None,
            width: None,
            height: None,
        };
        assert_eq!(
            manifest_rel_path(&icon),
            PathBuf::from("media/subreddit_icon.png")
        );
    }

    #[test]
    fn extension_falls_back_to_url_then_jpg() {
        let mut it = item("image", "abc", None);
        it.ext = None;
        it.url = "https://preview.redd.it/a.jpeg?s=1".into();
        assert_eq!(
            manifest_rel_path(&it),
            PathBuf::from("media/posts/abc.jpeg")
        );

        it.url = "https://example.com/download".into();
        assert_eq!(manifest_rel_path(&it), PathBuf::from("media/posts/abc.jpg"));
    }
}
