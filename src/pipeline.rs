//! Archive one listing: fetch, merge, save, download, render.

use crate::archive::{self, AboutFile, PostsFile};
use crate::client::{Api, Listing, PostsMode};
use crate::cookies::Cookies;
use crate::download::{Downloader, MediaReport};
use crate::html::{self, ViewerMeta};
use crate::http;
use crate::manifest::{self, Selection};
use crate::models::{MediaFormat, Sort, SubredditInfo, TimeFilter};
use crate::normalize;
use crate::target::{Target, TargetKind};
use crate::transform::TransformOptions;
use anyhow::{Context, Result, ensure};
use chrono::{NaiveDate, Utc};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Everything a run needs. Create with [`Config::new`] and adjust fields.
#[derive(Clone, Debug)]
pub struct Config {
    pub target: Target,
    /// Session cookies; empty for anonymous requests.
    pub cookies: Cookies,
    /// Replaces the emulated browser's User-Agent; should match the browser
    /// the cookies were exported from.
    pub user_agent: Option<String>,
    /// Output root; the archive is `<out_dir>/<r_|u_><name>/`.
    pub out_dir: PathBuf,
    pub posts: PostsMode,
    pub sort: Sort,
    /// Window for `top` and `controversial`.
    pub time: TimeFilter,
    /// Only keep fetched posts created on or after this day (UTC).
    pub since: Option<NaiveDate>,
    pub media: Selection,
    pub transforms: TransformOptions,
    /// Save the raw listing (`raw.json`).
    pub save_raw: bool,
    /// Download media; otherwise only JSON is written.
    pub download: bool,
    /// Write the offline viewer and the archive hub.
    pub offline: bool,
    /// Parallel media downloads.
    pub concurrency: usize,
    /// Pause between listing pages, to stay within Reddit's rate limits.
    pub page_delay: Duration,
    /// API origin (overridable for tests).
    pub api_base: String,
    /// Origin of original gallery images (overridable for tests).
    pub media_base: String,
}

impl Config {
    /// Defaults: first listing page, hot, all images (no videos), subreddit
    /// art, raw JSON, no viewer.
    pub fn new(target: Target) -> Self {
        Self {
            target,
            cookies: Cookies::default(),
            user_agent: None,
            out_dir: PathBuf::from("output"),
            posts: PostsMode::FirstPage,
            sort: Sort::Hot,
            time: TimeFilter::All,
            since: None,
            media: Selection {
                subreddit_art: true,
                ..Selection::default()
            },
            transforms: TransformOptions::default(),
            save_raw: true,
            download: true,
            offline: false,
            concurrency: 8,
            page_delay: Duration::from_millis(1200),
            api_base: "https://www.reddit.com".into(),
            media_base: "https://i.redd.it".into(),
        }
    }

    /// The archive directory of this target.
    pub fn archive_dir(&self) -> PathBuf {
        self.out_dir.join(self.target.dir_name())
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=100).contains(&self.transforms.quality),
            "JPEG quality must be between 1 and 100"
        );
        ensure!(self.concurrency > 0, "concurrency must be at least 1");
        Ok(())
    }
}

/// Outcome of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Posts in the archive after merging.
    pub posts: usize,
    /// Posts this run added.
    pub posts_new: usize,
    pub media: MediaReport,
}

/// Archive one listing. Configuration is validated before any network or
/// file access.
pub async fn run(cfg: &Config) -> Result<Summary> {
    cfg.validate()?;
    let http = http::client(cfg.user_agent.as_deref())?;
    let api = Api::new(http.clone(), &cfg.api_base, &cfg.cookies, cfg.page_delay);
    let dir = cfg.archive_dir();
    let auth = if cfg.cookies.is_empty() {
        "anonymous"
    } else {
        "cookies"
    };
    println!(
        "fetching {} (sort: {}, auth: {auth})",
        cfg.target,
        cfg.sort.as_str()
    );

    // Read the existing archive first: a corrupt file must stop the run
    // before anything is overwritten.
    let previous = archive::load_posts(&dir).await?;
    let about = fetch_about(&api, &cfg.target).await;
    let raw = api
        .listing(Listing {
            target: &cfg.target,
            sort: cfg.sort,
            time: cfg.time,
            posts: cfg.posts,
        })
        .await?;
    let since = cfg
        .since
        .map(|day| day.and_time(chrono::NaiveTime::MIN).and_utc().timestamp());
    let raw: Vec<_> = raw
        .into_iter()
        .filter(|post| since.is_none_or(|ts| normalize::created_utc(post) >= ts))
        .collect();
    let fetched = raw
        .iter()
        .map(|p| normalize::post(p, &cfg.media_base))
        .collect();

    let known: HashSet<String> = previous.iter().map(|p| p.id.clone()).collect();
    let posts = archive::merge_posts(fetched, previous);
    let posts_new = posts.iter().filter(|p| !known.contains(&p.id)).count();
    println!("posts: {} in archive ({posts_new} new)", posts.len());

    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let file = PostsFile {
        target: cfg.target.to_string(),
        sort: cfg.sort,
        time: cfg.time,
        fetched_at: Utc::now().to_rfc3339(),
        posts,
    };
    save_documents(cfg, &dir, &file, about.as_ref(), &raw).await?;
    let posts = &file.posts;

    let manifest = manifest::build(about.as_ref(), posts, &cfg.media, cfg.transforms.convert);
    let mut media = MediaReport {
        skipped: manifest.skipped_small,
        ..MediaReport::default()
    };
    describe_media_options(cfg, &manifest);
    if cfg.download {
        archive::write_json(&dir.join(archive::MANIFEST_FILE), &manifest.items).await?;
        let cookie = cfg.cookies.header_value();
        let downloader = Downloader::new(http, cookie, cfg.transforms, cfg.concurrency);
        media += downloader.run(&manifest.items, &dir).await;
    } else {
        println!("media downloads skipped");
    }

    if cfg.offline {
        let meta = ViewerMeta {
            sort: cfg.sort,
            time: cfg.time,
            fetched_at: &file.fetched_at,
        };
        let page = html::render_archive(&cfg.target, about.as_ref(), posts, &manifest.items, meta);
        let path = dir.join(archive::INDEX_FILE);
        archive::write_atomic(&path, page.as_bytes()).await?;
        println!("viewer: {}", path.display());

        let archives = archive::scan(&cfg.out_dir).await?;
        let hub = cfg.out_dir.join(archive::INDEX_FILE);
        archive::write_atomic(&hub, html::render_hub(&archives).as_bytes()).await?;
        println!("hub: {} ({} archives)", hub.display(), archives.len());
    }

    Ok(Summary {
        posts: posts.len(),
        posts_new,
        media,
    })
}

/// Subreddit metadata is optional: users and multis have none, and a
/// failure only costs the header and icon.
async fn fetch_about(api: &Api, target: &Target) -> Option<SubredditInfo> {
    if target.kind() != TargetKind::Subreddit || target.is_multi() {
        return None;
    }
    api.about(target)
        .await
        .inspect_err(|e| eprintln!("warning: subreddit info unavailable: {e:#}"))
        .ok()
}

async fn save_documents(
    cfg: &Config,
    dir: &Path,
    posts: &PostsFile,
    about: Option<&SubredditInfo>,
    raw: &[Value],
) -> Result<()> {
    if let Some(about) = about {
        let file = AboutFile {
            target: posts.target.clone(),
            fetched_at: posts.fetched_at.clone(),
            about: about.clone(),
        };
        archive::write_json(&dir.join(archive::ABOUT_FILE), &file).await?;
    }
    if cfg.save_raw {
        archive::write_json(&dir.join(archive::RAW_FILE), raw).await?;
    }
    archive::write_json(&dir.join(archive::POSTS_FILE), posts).await
}

fn describe_media_options(cfg: &Config, manifest: &manifest::Manifest) {
    let selection = &cfg.media;
    if !selection.formats.is_empty() {
        let formats: Vec<String> = selection.formats.iter().map(ToString::to_string).collect();
        println!("formats: {}", formats.join(", "));
    }
    if let Some(min) = selection.min_size {
        println!("min-size {min}: {} stills skipped", manifest.skipped_small);
    }
    let transforms = &cfg.transforms;
    if !transforms.is_noop() {
        let mut what = Vec::new();
        if let Some(format) = transforms.convert {
            what.push(format!("convert to {}", MediaFormat::from(format)));
        }
        if let Some(max) = transforms.max_size {
            what.push(format!("max size {max}"));
        }
        println!(
            "transforms: {} (JPEG/PNG/BMP stills, quality {})",
            what.join(", "),
            transforms.quality
        );
    }
    if cfg.download {
        println!("manifest: {} media files", manifest.items.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_configuration_fails_before_any_io() {
        let root = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(Target::subreddit("pics").unwrap());
        cfg.out_dir = root.path().join("archive");
        cfg.api_base = "http://127.0.0.1:1".into();
        cfg.transforms.quality = 0;
        let error = run(&cfg).await.unwrap_err().to_string();
        assert!(error.contains("quality"), "{error}");
        cfg.transforms.quality = 85;
        cfg.concurrency = 0;
        assert!(run(&cfg).await.is_err());
        assert!(!cfg.out_dir.exists());
    }
}
