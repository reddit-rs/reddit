//! Parallel media downloader with caching, retries and URL fallbacks.
//!
//! Files already in the archive are never requested again: every manifest item
//! is checked against the media directory before any HTTP request is made and
//! reported as [`DownloadReport::cached`]. Successful downloads are written to
//! a temporary `.part` file and renamed into place, so an interrupted run
//! cannot leave a truncated file behind that a later run would mistake for
//! cached content.

use crate::clean::{ext_from_url, format_from_mime, item_format};
use crate::models::{ManifestItem, MediaFormat};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use wreq::Client;
use wreq::header::{HeaderMap, HeaderValue};
use wreq_util::Emulation;

/// Outcome of a media download pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadReport {
    /// Manifest items considered.
    pub total: usize,
    /// Files fetched over the network this run.
    pub downloaded: usize,
    /// Files that were already present on disk and were not requested again.
    pub cached: usize,
    /// Files that could not be downloaded from any candidate URL.
    pub failed: usize,
    /// Downloaded files whose container format was rewritten (`--convert`).
    pub converted: usize,
    /// Downloaded files whose pixel dimensions were reduced (`--max-size`).
    pub resized: usize,
}

impl DownloadReport {
    /// Files that are now part of the archive (downloaded plus cached).
    pub fn present(&self) -> usize {
        self.downloaded + self.cached
    }
}

/// Opt-in, lossy image transforms applied while saving (see `--convert` and
/// `--max-size`). Files already on disk are never rewritten.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransformOptions {
    /// Downscale images so neither side exceeds this `(width, height)` box.
    pub max_size: Option<(u32, u32)>,
    /// Rewrite JPEG/PNG/BMP stills as this format.
    pub convert: Option<MediaFormat>,
    /// JPEG quality (1-100).
    pub quality: u8,
}

/// The result of one manifest item: fetched now, or already on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Downloaded { converted: bool, resized: bool },
    Cached,
}

/// Path of a manifest item relative to the `media/` directory
/// (e.g. `posts/abc123_00.jpg`).
fn file_name(item: &ManifestItem) -> PathBuf {
    let ext = item
        .ext
        .clone()
        .or_else(|| crate::clean::format_from_query(&item.url).map(|f| f.name().to_string()))
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

/// True when the item is already archived: a non-empty regular file.
async fn is_present(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// Sibling path used while a download is in flight (`<file>.part`).
fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
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

/// Download every manifest item that is not already in `<outdir>/media`.
///
/// Presence is checked before any request is made, so re-running an archive
/// costs no network traffic for media it already has.
pub async fn download_all(
    manifest: &[ManifestItem],
    outdir: &Path,
    ua: &str,
    cookie_header: Option<&str>,
    concurrency: usize,
    transforms: &TransformOptions,
) -> DownloadReport {
    let mut report = DownloadReport {
        total: manifest.len(),
        ..DownloadReport::default()
    };
    let client = Client::builder()
        .emulation(Emulation::Chrome131)
        .timeout(Duration::from_secs(120))
        .build()
        .expect("failed to build download client");
    let headers = match media_headers(ua, cookie_header) {
        Ok(h) => h,
        Err(e) => {
            println!("media headers invalid: {e}");
            report.failed = manifest.len();
            return report;
        }
    };
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let meddir = outdir.join("media");

    // Check the archive before going online: items that are already on disk
    // are never requested again.
    let mut pending = Vec::new();
    for item in manifest {
        if is_present(&meddir.join(file_name(item))).await {
            report.cached += 1;
        } else {
            pending.push(item.clone());
        }
    }
    if report.cached > 0 {
        println!(
            "media: {} of {} already cached, {} to fetch",
            report.cached,
            manifest.len(),
            pending.len()
        );
    }

    let total = pending.len();
    let mut handles = Vec::with_capacity(total);
    for item in pending {
        let client = client.clone();
        let sem = sem.clone();
        let meddir = meddir.clone();
        let headers = headers.clone();
        let transforms = *transforms;
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            download_one(&client, &item, &meddir, &headers, &transforms).await
        }));
    }

    let mut done = 0usize;
    for h in handles {
        done += 1;
        match h.await {
            Ok(Ok(Outcome::Downloaded { converted, resized })) => {
                report.downloaded += 1;
                report.converted += usize::from(converted);
                report.resized += usize::from(resized);
                if done.is_multiple_of(25) {
                    println!("[{done}/{total}] downloaded");
                }
            }
            Ok(Ok(Outcome::Cached)) => report.cached += 1,
            Ok(Err(e)) => {
                report.failed += 1;
                println!("[{done}/{total}] FAIL {e}");
            }
            Err(e) => {
                report.failed += 1;
                println!("[{done}/{total}] FAIL task error: {e}");
            }
        }
    }
    let mut extra = Vec::new();
    if report.converted > 0 {
        extra.push(format!("{} converted", report.converted));
    }
    if report.resized > 0 {
        extra.push(format!("{} resized", report.resized));
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(" ({})", extra.join(", "))
    };
    println!(
        "\nmedia: {} downloaded{extra}, {} cached, {} failed",
        report.downloaded, report.cached, report.failed
    );
    report
}

async fn download_one(
    client: &Client,
    item: &ManifestItem,
    meddir: &Path,
    headers: &HeaderMap,
    transforms: &TransformOptions,
) -> Result<Outcome> {
    let rel = file_name(item);
    let path = meddir.join(&rel);
    if is_present(&path).await {
        return Ok(Outcome::Cached);
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut urls = vec![item.url.clone()];
    if let Some(f) = &item.fallback
        && f != &item.url
    {
        urls.push(f.clone());
    }
    // When the file will be re-encoded, the server's content type says nothing
    // about the final format and must not be checked against it.
    let converting = transforms.convert.is_some() && crate::clean::is_post_still(item);

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
                // Never store a fallback of the wrong format under this file
                // name (e.g. a jpg preview saved as `.gif`). Unknown content
                // types are accepted; only known mismatches are rejected.
                if !converting && let Some(expected) = item_format(item) {
                    let actual = resp
                        .headers()
                        .get(wreq::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(format_from_mime);
                    if let Some(actual) = actual
                        && actual != expected
                    {
                        println!(
                            "  skipping {}: server sent {} (expected .{})",
                            rel.display(),
                            actual.name(),
                            expected.name()
                        );
                        break; // try the next candidate URL
                    }
                }
                let bytes = match resp.bytes().await {
                    Ok(b) => b,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                        continue;
                    }
                };
                if !bytes.is_empty() {
                    // Optional lossy transforms (convert / downscale). A
                    // failure here is reported instead of writing content
                    // that would not match its file name.
                    match crate::transform::apply(&bytes, item, transforms) {
                        Ok(crate::transform::Applied::Unchanged) => {
                            write_atomic(&path, &bytes).await?;
                            return Ok(Outcome::Downloaded {
                                converted: false,
                                resized: false,
                            });
                        }
                        Ok(crate::transform::Applied::Transformed {
                            bytes: out,
                            converted,
                            resized,
                        }) => {
                            write_atomic(&path, &out).await?;
                            return Ok(Outcome::Downloaded { converted, resized });
                        }
                        Err(e) => {
                            return Err(e.context(format!(
                                "transforming {} (download happened; file not saved)",
                                rel.display()
                            )));
                        }
                    }
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
    Err(anyhow::anyhow!("{}", rel.display()))
}

/// Write `bytes` to `path` via a temporary sibling, so readers (and later
/// runs) only ever see a complete file.
async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = part_path(path);
    let res = async {
        let mut f = tokio::fs::File::create(&tmp).await?;
        f.write_all(bytes).await?;
        f.flush().await?;
        drop(f);
        tokio::fs::rename(&tmp, path).await
    }
    .await;
    if res.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    Ok(res?)
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

    #[test]
    fn format_query_decides_the_file_extension() {
        let mut it = item("image", "abc", None);
        it.ext = None;
        it.url = "https://external-preview.redd.it/x.png?format=pjpg&auto=webp&s=1".into();
        assert_eq!(manifest_rel_path(&it), PathBuf::from("media/posts/abc.jpg"));
    }

    #[test]
    fn part_path_is_a_sibling() {
        assert_eq!(
            part_path(Path::new("/a/media/posts/x.jpg")),
            PathBuf::from("/a/media/posts/x.jpg.part")
        );
    }

    #[tokio::test]
    async fn presence_requires_a_nonempty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.jpg");
        assert!(!is_present(&path).await);
        tokio::fs::write(&path, b"").await.unwrap();
        assert!(!is_present(&path).await, "empty file must be re-downloaded");
        tokio::fs::write(&path, b"x").await.unwrap();
        assert!(is_present(&path).await);
        assert!(!is_present(dir.path()).await, "directories are not files");
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jpg");
        write_atomic(&path, b"data").await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"data");
        assert!(!part_path(&path).exists());
    }
}
