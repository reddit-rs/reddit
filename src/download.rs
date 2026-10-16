//! Parallel media downloads with caching, retries and URL fallbacks.
//!
//! A manifest item whose file already exists (non-empty) is never requested
//! again. New files are written atomically, so an interrupted run cannot
//! leave a truncated file that a later run would mistake for cached content.

use crate::archive::write_atomic;
use crate::http;
use crate::models::{ManifestItem, MediaFormat};
use crate::transform::{self, Applied, TransformOptions};
use anyhow::{Context, Result, anyhow, bail};
use std::ops::AddAssign;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use wreq::StatusCode;
use wreq::header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, REFERER};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Media statistics of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaReport {
    /// Files selected by the manifest.
    pub total: usize,
    /// Files fetched over the network.
    pub downloaded: usize,
    /// Files already on disk, not requested again.
    pub cached: usize,
    /// Files no candidate URL could provide.
    pub failed: usize,
    /// Downloads re-encoded by `--convert`.
    pub converted: usize,
    /// Downloads shrunk by `--max-size`.
    pub resized: usize,
    /// Stills dropped by `--min-size` before downloading.
    pub skipped: usize,
}

impl AddAssign for MediaReport {
    fn add_assign(&mut self, other: Self) {
        self.total += other.total;
        self.downloaded += other.downloaded;
        self.cached += other.cached;
        self.failed += other.failed;
        self.converted += other.converted;
        self.resized += other.resized;
        self.skipped += other.skipped;
    }
}

/// Downloads manifest items into an archive directory.
pub(crate) struct Downloader {
    http: wreq::Client,
    headers: HeaderMap,
    transforms: TransformOptions,
    concurrency: usize,
}

struct Saved {
    converted: bool,
    resized: bool,
}

impl Downloader {
    /// `cookie` is only sent to HTTPS Reddit hosts.
    pub fn new(
        http: wreq::Client,
        cookie: Option<HeaderValue>,
        transforms: TransformOptions,
        concurrency: usize,
    ) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert(REFERER, HeaderValue::from_static("https://www.reddit.com/"));
        if let Some(cookie) = cookie {
            headers.insert(COOKIE, cookie);
        }
        Self {
            http,
            headers,
            transforms,
            concurrency: concurrency.max(1),
        }
    }

    /// Download every item missing from `archive_dir`.
    pub async fn run(self, items: &[ManifestItem], archive_dir: &Path) -> MediaReport {
        let mut report = MediaReport {
            total: items.len(),
            ..MediaReport::default()
        };
        let mut pending = Vec::new();
        for item in items {
            if is_present(&archive_dir.join(item.path())).await {
                report.cached += 1;
            } else {
                pending.push(item.clone());
            }
        }
        if pending.is_empty() {
            return report;
        }
        println!(
            "media: {} cached, {} to download",
            report.cached,
            pending.len()
        );

        let this = Arc::new(self);
        let permits = Arc::new(Semaphore::new(this.concurrency));
        let mut tasks = JoinSet::new();
        for item in pending {
            let (this, permits, dir) = (this.clone(), permits.clone(), archive_dir.to_owned());
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .expect("semaphore is never closed");
                let result = this.fetch(&item, &dir).await;
                (item, result)
            });
        }

        let todo = tasks.len();
        let mut done = 0;
        while let Some(joined) = tasks.join_next().await {
            done += 1;
            match joined {
                Ok((_, Ok(saved))) => {
                    report.downloaded += 1;
                    report.converted += usize::from(saved.converted);
                    report.resized += usize::from(saved.resized);
                }
                Ok((item, Err(e))) => {
                    report.failed += 1;
                    eprintln!("  failed {}: {e:#}", item.web_path());
                }
                Err(e) => {
                    report.failed += 1;
                    eprintln!("  download task failed: {e}");
                }
            }
            if done % 25 == 0 || done == todo {
                println!("  [{done}/{todo}] media processed");
            }
        }
        report
    }

    /// Try the item's URL, then its fallback.
    async fn fetch(&self, item: &ManifestItem, archive_dir: &Path) -> Result<Saved> {
        let path = archive_dir.join(item.path());
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut last_error = anyhow!("no URL");
        for url in std::iter::once(&item.url).chain(&item.fallback) {
            match self.fetch_url(item, url).await {
                Ok(data) => return self.save(item, data, &path).await,
                Err(e) => last_error = e.context(url.clone()),
            }
        }
        Err(last_error)
    }

    /// Body of `url`, retrying transient failures. Permanent failures and
    /// content of the wrong format fail immediately so the next URL is tried.
    async fn fetch_url(&self, item: &ManifestItem, url: &str) -> Result<Vec<u8>> {
        let mut headers = self.headers.clone();
        if !http::is_secure_reddit_url(url) {
            headers.remove(COOKIE);
        }
        let mut last_error = anyhow!("no request made");
        let mut wait = Duration::ZERO;
        for attempt in 0..http::ATTEMPTS {
            tokio::time::sleep(wait).await;
            wait = http::backoff(attempt);
            let response = match self
                .http
                .get(url)
                .headers(headers.clone())
                .timeout(REQUEST_TIMEOUT)
                .send()
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    last_error = anyhow!("network error: {e}");
                    continue;
                }
            };
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS {
                wait = http::rate_limit_wait(response.headers(), attempt);
                println!("  media rate limited, waiting up to {}s", wait.as_secs());
                last_error = anyhow!("HTTP 429");
                continue;
            }
            if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
                last_error = anyhow!("HTTP {status}");
                continue;
            }
            if !status.is_success() {
                bail!("HTTP {status}");
            }
            let declared = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(MediaFormat::from_mime);
            let data = match response.bytes().await {
                Ok(data) if data.is_empty() => bail!("empty response"),
                Ok(data) => data.to_vec(),
                Err(e) => {
                    last_error = anyhow!("interrupted download: {e}");
                    continue;
                }
            };
            self.check_format(item, transform::sniff(&data).or(declared))?;
            return Ok(data);
        }
        Err(last_error)
    }

    /// Never store content under a file name of another format (e.g. a JPEG
    /// preview as `.gif`). Unknown formats are accepted.
    fn check_format(&self, item: &ManifestItem, actual: Option<MediaFormat>) -> Result<()> {
        let (Some(expected), Some(actual)) = (item.format, actual) else {
            return Ok(());
        };
        let converting = self.transforms.convert.is_some() && self.transforms.applies_to(item);
        let acceptable = if converting {
            actual.is_static_image()
        } else {
            actual == expected
        };
        if !acceptable {
            bail!("server sent {actual}, expected {expected}");
        }
        Ok(())
    }

    async fn save(&self, item: &ManifestItem, data: Vec<u8>, path: &Path) -> Result<Saved> {
        let (item, transforms) = (item.clone(), self.transforms);
        let (data, applied) = tokio::task::spawn_blocking(move || {
            let applied = transform::apply(&data, &item, &transforms);
            (data, applied)
        })
        .await
        .context("transform task failed")?;
        let saved = match applied.context("cannot transform image")? {
            Applied::Unchanged => {
                write_atomic(path, &data).await?;
                Saved {
                    converted: false,
                    resized: false,
                }
            }
            Applied::Transformed {
                bytes,
                converted,
                resized,
            } => {
                write_atomic(path, &bytes).await?;
                Saved { converted, resized }
            }
        };
        Ok(saved)
    }
}

/// A non-empty regular file counts as archived.
async fn is_present(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|m| m.is_file() && m.len() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn presence_requires_a_non_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.jpg");
        assert!(!is_present(&path).await);
        std::fs::write(&path, b"").unwrap();
        assert!(!is_present(&path).await, "empty files are re-downloaded");
        std::fs::write(&path, b"x").unwrap();
        assert!(is_present(&path).await);
        assert!(!is_present(dir.path()).await, "directories are not files");
    }

    #[test]
    fn reports_add_up() {
        let one = MediaReport {
            total: 2,
            downloaded: 1,
            cached: 1,
            ..MediaReport::default()
        };
        let mut sum = one;
        sum += one;
        assert_eq!((sum.total, sum.downloaded, sum.cached), (4, 2, 2));
    }
}
