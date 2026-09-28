//! Offline viewer for reddit: download a subreddit's posts as structured JSON
//! plus media, and optionally generate a browsable offline `index.html`.
//!
//! The pipeline lives in [`run`]; [`Config`] selects the target, listing sort,
//! pagination depth, cookies and output directory.

pub mod clean;
pub mod client;
pub mod download;
pub mod html;
pub mod models;

pub use crate::models::{
    GalleryItem, ManifestItem, MediaAsset, Post, Sort, SubredditInfo, Target, TargetKind,
    TimeFilter, VideoInfo,
};

use crate::client::Client;
use crate::download::download_all;
use anyhow::{Result, bail};
use chrono::Utc;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Browser User-Agent used by default. Reddit binds sessions to the browser
/// fingerprint, so this should match the browser the cookies were exported
/// from (override with `--user-agent`).
pub const UA_DEFAULT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36";

/// How much of a listing to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostsMode {
    /// Only the first listing page (reddit's default 25 posts). Default.
    Snapshot,
    /// Paginate; `0` = everything.
    All(u64),
}

impl PostsMode {
    /// `limit` sent to the listing API for each page (reddit caps it at 100).
    pub fn page_size(self) -> u64 {
        match self {
            PostsMode::Snapshot => 25,
            PostsMode::All(0) => 100,
            PostsMode::All(n) => n.min(100),
        }
    }

    /// Total post cap; `None` = unlimited.
    pub fn cap(self) -> Option<u64> {
        match self {
            PostsMode::Snapshot => Some(25),
            PostsMode::All(0) => None,
            PostsMode::All(n) => Some(n),
        }
    }

    /// How many listing pages to fetch at most. A snapshot is exactly what the
    /// first page load shows; every other mode paginates until the cap.
    pub fn max_pages(self) -> usize {
        match self {
            PostsMode::Snapshot => 1,
            PostsMode::All(_) => crate::client::MAX_PAGES,
        }
    }
}

/// A validated listing target plus optional listing hints parsed from a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTarget {
    pub target: Target,
    pub sort: Option<Sort>,
    pub time: Option<TimeFilter>,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Subreddit or user to fetch.
    pub target: Target,
    /// `'k=v; k2=v2'` string or a path to a Netscape-format cookies.txt file.
    pub cookies: Option<String>,
    /// User-Agent header; should match the browser the cookies came from.
    pub user_agent: String,
    /// Base output directory; results land in `<out_dir>/<r_|u_><name>/`.
    pub out_dir: PathBuf,
    /// How much of the listing to fetch ([`PostsMode`]).
    pub posts: PostsMode,
    /// Listing sort order.
    pub sort: Sort,
    /// Time window for `top` / `controversial`.
    pub time: TimeFilter,
    /// Also download reddit-hosted video files (images only by default).
    pub videos: bool,
    /// Max images per gallery (`0` = all).
    pub gallery_images: usize,
    /// Only keep posts created on/after this date (YYYY-MM-DD).
    pub since: Option<String>,
    /// Skip the subreddit icon/banner download.
    pub skip_icon: bool,
    /// Skip the raw listing JSON dump.
    pub no_raw: bool,
    /// Skip media downloads entirely (JSON only).
    pub no_downloads: bool,
    /// Generate a self-contained `index.html` offline viewer.
    pub offline: bool,
    /// API base URL override (mainly for tests).
    pub base_url: String,
    /// Image host used to derive original gallery URLs (mainly for tests).
    pub media_base: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target: Target::subreddit(String::new()),
            cookies: None,
            user_agent: UA_DEFAULT.to_string(),
            out_dir: PathBuf::from("output"),
            posts: PostsMode::Snapshot,
            sort: Sort::Hot,
            time: TimeFilter::All,
            videos: false,
            gallery_images: 0,
            since: None,
            skip_icon: false,
            no_raw: false,
            no_downloads: false,
            offline: false,
            base_url: "https://www.reddit.com".to_string(),
            media_base: "https://i.redd.it".to_string(),
        }
    }
}

/// Result of a run.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub posts: usize,
    pub media_total: usize,
    pub media_failed: usize,
}

fn valid_subreddit(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('+').all(|part| {
            !part.is_empty()
                && part.len() <= 32
                && part
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

fn valid_user(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn validate_target(target: &Target) -> Result<()> {
    let ok = match target.kind {
        TargetKind::Subreddit => valid_subreddit(&target.name),
        TargetKind::User => valid_user(&target.name),
    };
    if !ok {
        bail!("invalid {} name: '{}'", target.kind.name(), target.name);
    }
    Ok(())
}

/// Parse a subreddit/user name or URL. Accepts bare names (`rust`), reddit
/// paths (`r/rust`, `u/spez`), and full URLs
/// (`https://www.reddit.com/r/rust/top/?t=week`), including multi-subreddits
/// (`r/rust+golang`). Sort and time hints from the URL are returned alongside.
pub fn parse_target(s: &str) -> Result<ParsedTarget> {
    let raw = s.trim();
    if raw.is_empty() {
        bail!("no subreddit or user given");
    }
    let (before_query, query) = raw.split_once('?').unwrap_or((raw, ""));
    let clean = before_query.trim_end_matches('/');

    let path = if clean.starts_with("http://") || clean.starts_with("https://") {
        let rest = clean.split_once("://").map(|(_, r)| r).unwrap_or(clean);
        match rest.find('/') {
            Some(i) => &rest[i..],
            None => "",
        }
    } else if let Some((host, rest)) = clean.split_once('/')
        && host.contains('.')
    {
        rest
    } else {
        clean
    };

    let segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (kind, name, rest): (TargetKind, &str, &[&str]) = match segs.as_slice() {
        ["r" | "R", name, rest @ ..] => (TargetKind::Subreddit, name, rest),
        ["u" | "U" | "user" | "User", name, rest @ ..] => (TargetKind::User, name, rest),
        ["r" | "R" | "u" | "U" | "user" | "User"] => bail!("missing name in '{s}'"),
        [name, rest @ ..] => (TargetKind::Subreddit, name, rest),
        [] => bail!("no subreddit or user in '{s}'"),
    };

    let target = Target {
        kind,
        name: name.to_string(),
    };
    validate_target(&target)?;

    let mut sort = rest
        .first()
        .and_then(|s| Sort::from_name(&s.to_lowercase()));
    let mut time = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = v.to_lowercase();
        match k.to_lowercase().as_str() {
            "sort" => sort = sort.or_else(|| Sort::from_name(&v)),
            "t" => time = TimeFilter::from_name(&v),
            _ => {}
        }
    }

    Ok(ParsedTarget { target, sort, time })
}

async fn save_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let s = serde_json::to_string_pretty(value)?;
    tokio::fs::write(path, s).await?;
    Ok(())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Run the pipeline: fetch the listing, save JSON, download media and
/// optionally render the offline viewer. All network and file IO happens here.
pub async fn run(cfg: Config) -> Result<Summary> {
    validate_target(&cfg.target)?;

    let cookies: HashMap<String, String> = match &cfg.cookies {
        Some(c) => {
            let parsed = clean::parse_cookies(c);
            if parsed.is_empty() {
                println!("  warning: no cookies parsed from '{c}' (missing file?)");
            }
            parsed
        }
        None => HashMap::new(),
    };

    let outdir = cfg.out_dir.join(cfg.target.output_name());
    tokio::fs::create_dir_all(&outdir).await?;

    let display = cfg.target.display();
    println!(
        "fetching {display} (sort: {}, auth: {})",
        cfg.sort.name(),
        if cookies.is_empty() {
            "anonymous"
        } else {
            "cookies"
        }
    );

    let mut client = Client::with_base(&cfg.user_agent, &cookies, &cfg.base_url)?;
    client.page_delay_ms = 1200;

    // Subreddit metadata (not available for multi-subreddit targets).
    let about = if cfg.target.kind == TargetKind::Subreddit && !cfg.target.is_multi() {
        match client.get_about(&cfg.target).await {
            Ok(v) => Some(clean::clean_about(&v)),
            Err(e) => {
                println!("  warning: subreddit info unavailable: {e}");
                None
            }
        }
    } else {
        None
    };

    let raw_items = client
        .get_listing(
            &cfg.target,
            cfg.sort,
            cfg.time,
            cfg.posts.cap(),
            cfg.posts.page_size(),
            cfg.posts.max_pages(),
        )
        .await?;

    let raw_items: Vec<Value> = match &cfg.since {
        Some(since) => {
            let ts = clean::parse_since(since)?;
            raw_items
                .into_iter()
                .filter(|d| clean::get_f64(d, "created_utc").unwrap_or(0.0) as i64 >= ts)
                .collect()
        }
        None => raw_items,
    };
    let posts: Vec<Post> = raw_items
        .iter()
        .map(|d| clean::clean_post(d, &cfg.media_base))
        .collect();
    println!("posts: {}", posts.len());

    let fetched_at = now_iso();

    if let Some(a) = &about {
        save_json(
            &outdir.join(format!("{}_about.json", cfg.target.name)),
            &json!({
                "target": display,
                "fetched_at": fetched_at,
                "about": a,
            }),
        )
        .await?;
    }

    if !cfg.no_raw {
        save_json(
            &outdir.join(format!("{}_posts_raw.json", cfg.target.name)),
            &json!(raw_items),
        )
        .await?;
    }

    save_json(
        &outdir.join(format!("{}_posts.json", cfg.target.name)),
        &json!({
            "target": display,
            "kind": cfg.target.kind.name(),
            "sort": cfg.sort.name(),
            "time": cfg.time.name(),
            "fetched_at": fetched_at,
            "total": posts.len(),
            "posts": posts,
        }),
    )
    .await?;
    println!(
        "saved {}_posts.json ({} posts)",
        cfg.target.name,
        posts.len()
    );

    let manifest = clean::build_manifest(
        about.as_ref(),
        &posts,
        cfg.videos,
        cfg.gallery_images,
        cfg.skip_icon,
    );

    let mut media_total = 0;
    let mut media_failed = 0;
    if !cfg.no_downloads {
        save_json(
            &outdir.join("media_manifest.json"),
            &serde_json::to_value(&manifest)?,
        )
        .await?;
        println!("manifest: {} media files", manifest.len());
        let (ok, failed) = download_all(
            &manifest,
            &outdir,
            &cfg.user_agent,
            client.cookie_header(),
            8,
        )
        .await;
        media_total = ok;
        media_failed = failed;
    } else {
        println!("media downloads skipped (--no-downloads)");
    }

    if cfg.offline {
        let page = html::render_index(
            &cfg.target,
            about.as_ref(),
            &posts,
            &manifest,
            &html::ViewerMeta {
                sort: cfg.sort,
                time: cfg.time,
                fetched_at: &fetched_at,
            },
        );
        tokio::fs::write(outdir.join("index.html"), page).await?;
        println!("saved index.html (offline viewer, {} posts)", posts.len());
    }

    Ok(Summary {
        posts: posts.len(),
        media_total,
        media_failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Target {
        parse_target(s).unwrap().target
    }

    #[test]
    fn bare_names_and_paths() {
        assert_eq!(t("funny"), Target::subreddit("funny"));
        assert_eq!(t("r/rust"), Target::subreddit("rust"));
        assert_eq!(t("/r/rust/"), Target::subreddit("rust"));
        assert_eq!(t("reddit.com/r/rust"), Target::subreddit("rust"));
        assert_eq!(t("u/spez"), Target::user("spez"));
        assert_eq!(t("user/spez/submitted"), Target::user("spez"));
        assert_eq!(t("https://www.reddit.com/user/spez"), Target::user("spez"));
    }

    #[test]
    fn full_urls_with_sort_and_time_hints() {
        let p = parse_target("https://www.reddit.com/r/rust").unwrap();
        assert_eq!(p.target, Target::subreddit("rust"));
        assert_eq!(p.sort, None);

        let p = parse_target("https://old.reddit.com/r/rust/top/?t=week").unwrap();
        assert_eq!(p.target, Target::subreddit("rust"));
        assert_eq!(p.sort, Some(Sort::Top));
        assert_eq!(p.time, Some(TimeFilter::Week));

        let p = parse_target("https://www.reddit.com/r/rust/new").unwrap();
        assert_eq!(p.sort, Some(Sort::New));

        let p = parse_target("https://www.reddit.com/r/rust/new?sort=controversial").unwrap();
        assert_eq!(p.sort, Some(Sort::New)); // path hint wins over query
    }

    #[test]
    fn multi_subreddits() {
        let target = t("r/rust+golang/new");
        assert_eq!(target.name, "rust+golang");
        assert!(target.is_multi());
        assert_eq!(target.output_name(), "r_rust+golang");
    }

    #[test]
    fn comments_urls_use_the_subreddit() {
        let target = t("https://www.reddit.com/r/rust/comments/abc123/some_title/");
        assert_eq!(target, Target::subreddit("rust"));
    }

    #[test]
    fn invalid_targets_rejected() {
        for bad in [
            "",
            "https://www.reddit.com/",
            "r/",
            "r/bad name!",
            "r/../etc",
            "u/x",
        ] {
            assert!(parse_target(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn target_display_and_output() {
        let target = Target::subreddit("funny");
        assert_eq!(target.display(), "r/funny");
        assert_eq!(target.output_name(), "r_funny");
        assert_eq!(target.url(), "https://www.reddit.com/r/funny/");
        assert_eq!(Target::user("spez").output_name(), "u_spez",);
    }

    #[test]
    fn posts_mode_sizes() {
        assert_eq!(PostsMode::Snapshot.page_size(), 25);
        assert_eq!(PostsMode::Snapshot.cap(), Some(25));
        assert_eq!(PostsMode::Snapshot.max_pages(), 1);
        assert_eq!(PostsMode::All(0).page_size(), 100);
        assert_eq!(PostsMode::All(0).cap(), None);
        assert_eq!(PostsMode::All(0).max_pages(), crate::client::MAX_PAGES);
        assert_eq!(PostsMode::All(7).page_size(), 7);
        assert_eq!(PostsMode::All(7).max_pages(), crate::client::MAX_PAGES);
        assert_eq!(PostsMode::All(500).page_size(), 100);
        assert_eq!(PostsMode::All(500).cap(), Some(500));
    }
}
