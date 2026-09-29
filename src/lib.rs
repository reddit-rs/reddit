//! Offline viewer for reddit: download a subreddit's posts as structured JSON
//! plus media, and optionally generate a browsable offline `index.html`.
//!
//! The pipeline lives in [`run`]; [`Config`] selects the target, listing sort,
//! pagination depth, cookies and output directory. Runs are incremental: a
//! fresh listing is merged into the existing archive and media files that are
//! already on disk are never requested again.

pub mod clean;
pub mod client;
pub mod download;
pub mod html;
pub mod models;
pub mod transform;

pub use crate::download::{DownloadReport, TransformOptions};
pub use crate::models::{
    GalleryItem, ManifestItem, MediaAsset, MediaFormat, Post, Sort, SubredditInfo, Target,
    TargetKind, TimeFilter, VideoInfo,
};

use crate::client::Client;
use crate::download::download_all;
use anyhow::{Result, bail};
use chrono::Utc;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
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
    /// Only download these media formats ([`MediaFormat`]; `jpeg` counts as
    /// `jpg`). Empty = every format.
    pub formats: Vec<MediaFormat>,
    /// Skip still images whose short side is smaller than `.0` or whose long
    /// side is smaller than `.1` (e.g. `(768, 1024)` keeps portrait images
    /// ≥768×1024 and landscape images ≥1024×768). `None` = no floor.
    pub min_size: Option<(u32, u32)>,
    /// Downscale still images so neither side exceeds this `(width, height)`
    /// box; never upscales. `None` = no cap.
    pub max_size: Option<(u32, u32)>,
    /// Convert JPEG/PNG/BMP stills to this format (`Jpg` or `Png`). GIF/WebP
    /// can be animated and are kept as-is.
    pub convert: Option<MediaFormat>,
    /// JPEG quality for converted/resized images (1-100).
    pub quality: u8,
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
            formats: Vec::new(),
            min_size: None,
            max_size: None,
            convert: None,
            quality: 85,
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
    /// Posts in the archive after merging with previous runs.
    pub posts: usize,
    /// Posts added to the archive by this run.
    pub posts_new: usize,
    /// Media files the manifest asked for.
    pub media_total: usize,
    /// Media files fetched over the network by this run.
    pub media_downloaded: usize,
    /// Media files that were already on disk (not re-downloaded).
    pub media_cached: usize,
    /// Media files that could not be downloaded.
    pub media_failed: usize,
    /// Downloaded files rewritten to the `--convert` format.
    pub media_converted: usize,
    /// Downloaded files downscaled by `--max-size`.
    pub media_resized: usize,
    /// Manifest items dropped by `--min-size`.
    pub media_skipped: usize,
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

async fn read_json(path: &Path) -> Option<Value> {
    let s = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&s).ok()
}

/// Posts stored by a previous run (`<name>_posts.json`), if any.
async fn load_posts(path: &Path) -> Vec<Post> {
    let Ok(s) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    let posts = serde_json::from_str::<Value>(&s)
        .ok()
        .and_then(|v| v.get("posts").cloned())
        .and_then(|p| serde_json::from_value::<Vec<Post>>(p).ok());
    match posts {
        Some(posts) => posts,
        None => {
            println!(
                "  warning: could not read {} — starting a new archive",
                path.display()
            );
            Vec::new()
        }
    }
}

/// Merge a freshly fetched listing into an existing archive: posts fetched now
/// keep their (fresher) data and listing order, posts archived earlier are
/// appended newest first, duplicates are dropped. A re-run therefore never
/// drops content it already has.
fn merge_posts(fetched: Vec<Post>, previous: Vec<Post>) -> Vec<Post> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Post> = Vec::with_capacity(fetched.len() + previous.len());

    for p in fetched {
        if p.id.is_empty() || !seen.insert(p.id.clone()) {
            continue;
        }
        out.push(p);
    }

    let mut older: Vec<Post> = Vec::new();
    for p in previous {
        if p.id.is_empty() || !seen.insert(p.id.clone()) {
            continue;
        }
        older.push(p);
    }
    older.sort_by(|a, b| {
        b.created_utc
            .cmp(&a.created_utc)
            .then_with(|| a.id.cmp(&b.id))
    });
    out.extend(older);
    out
}

/// Local `media/subreddit_icon.*` path, relative to the output root.
fn local_icon(dir: &Path, rel_dir: &str) -> Option<String> {
    for entry in std::fs::read_dir(dir.join("media")).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("subreddit_icon.") {
            return Some(format!("{rel_dir}/media/{name}"));
        }
    }
    None
}

/// Describe every `r_*` / `u_*` archive directory under `root`.
async fn scan_archives(root: &Path) -> Vec<html::ArchiveEntry> {
    let mut entries = Vec::new();
    let Ok(mut dir) = tokio::fs::read_dir(root).await else {
        return entries;
    };
    while let Ok(Some(entry)) = dir.next_entry().await {
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (prefix, sub) = match name.strip_prefix("r_") {
            Some(sub) => ("r/", sub),
            None => match name.strip_prefix("u_") {
                Some(sub) => ("u/", sub),
                None => continue,
            },
        };
        if sub.is_empty() {
            continue;
        }

        let path = entry.path();
        let about: Option<SubredditInfo> = read_json(&path.join(format!("{sub}_about.json")))
            .await
            .and_then(|v| v.get("about").cloned())
            .and_then(|a| serde_json::from_value(a).ok());
        let posts_json = read_json(&path.join(format!("{sub}_posts.json"))).await;

        entries.push(html::ArchiveEntry {
            dir: name.clone(),
            display: posts_json
                .as_ref()
                .and_then(|v| v.get("target"))
                .and_then(|t| t.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{prefix}{sub}")),
            title: about.as_ref().and_then(|a| a.title.clone()),
            icon: local_icon(&path, &name),
            icon_remote: about.as_ref().and_then(|a| a.icon.clone()),
            posts: posts_json
                .as_ref()
                .and_then(|v| v.get("posts"))
                .and_then(|p| p.as_array())
                .map(Vec::len)
                .unwrap_or(0),
            fetched_at: posts_json
                .as_ref()
                .and_then(|v| v.get("fetched_at"))
                .and_then(|t| t.as_str())
                .map(str::to_string),
            over18: about.as_ref().is_some_and(|a| a.over_18),
            viewer: path.join("index.html").is_file(),
        });
    }
    entries.sort_by(|a, b| a.dir.cmp(&b.dir));
    entries
}

/// Write the output root `index.html` listing every archive. Returns how many
/// archives it links to.
async fn write_archive_hub(root: &Path) -> Result<usize> {
    let entries = scan_archives(root).await;
    let count = entries.len();
    tokio::fs::write(root.join("index.html"), html::render_hub(&entries)).await?;
    Ok(count)
}

/// Run the pipeline: fetch the listing, save JSON, download media and
/// optionally render the offline viewer. All network and file IO happens here.
pub async fn run(cfg: Config) -> Result<Summary> {
    validate_target(&cfg.target)?;
    if let Some(convert) = cfg.convert
        && !matches!(convert, MediaFormat::Jpg | MediaFormat::Png)
    {
        bail!(
            "conversion target must be jpg or png, not '{}'",
            convert.name()
        );
    }

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
    let fetched: Vec<Post> = raw_items
        .iter()
        .map(|d| clean::clean_post(d, &cfg.media_base))
        .collect();

    // Merge into the existing archive: posts saved by earlier runs stay in the
    // JSON (and their media is not requested again below).
    let posts_path = outdir.join(format!("{}_posts.json", cfg.target.name));
    let previous = load_posts(&posts_path).await;
    let known: HashSet<String> = previous.iter().map(|p| p.id.clone()).collect();
    let posts = merge_posts(fetched, previous);
    let posts_new = posts.iter().filter(|p| !known.contains(&p.id)).count();
    println!("posts: {} in archive ({posts_new} new)", posts.len());

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
        cfg.videos || cfg.formats.iter().any(|f| f.is_video()),
        cfg.gallery_images,
        cfg.skip_icon,
    );
    if !cfg.formats.is_empty() {
        let formats = cfg
            .formats
            .iter()
            .map(|f| f.name())
            .collect::<Vec<_>>()
            .join(", ");
        println!("formats: {formats}");
    }
    let manifest = clean::filter_manifest(manifest, &cfg.formats);
    let before_min_size = manifest.len();
    let manifest = clean::filter_manifest_min_size(manifest, cfg.min_size);
    let skipped_small = before_min_size - manifest.len();
    if let Some((short, long)) = cfg.min_size {
        println!(
            "min-size: stills need short side >= {short} and long side >= {long}; {skipped_small} skipped"
        );
    }
    // Rewrite extensions before anything is fetched so file names, the
    // manifest and the viewer agree.
    let manifest = clean::apply_output_format(manifest, cfg.convert);
    let transforms = TransformOptions {
        max_size: cfg.max_size,
        convert: cfg.convert,
        quality: cfg.quality,
    };
    if cfg.convert.is_some() || cfg.max_size.is_some() {
        let mut what = Vec::new();
        if let Some(convert) = cfg.convert {
            what.push(format!("convert to {}", convert.name()));
        }
        if let Some((width, height)) = cfg.max_size {
            what.push(format!("max size {width}x{height}"));
        }
        println!(
            "image transforms: {} (JPEG/PNG/BMP stills only, quality {})",
            what.join(", "),
            cfg.quality
        );
    }

    let mut report = DownloadReport::default();
    if !cfg.no_downloads {
        save_json(
            &outdir.join("media_manifest.json"),
            &serde_json::to_value(&manifest)?,
        )
        .await?;
        println!("manifest: {} media files", manifest.len());
        report = download_all(
            &manifest,
            &outdir,
            &cfg.user_agent,
            client.cookie_header(),
            8,
            &transforms,
        )
        .await;
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
                hub: true,
            },
        );
        let page_path = outdir.join("index.html");
        tokio::fs::write(&page_path, page).await?;
        println!(
            "saved {} (offline viewer, {} posts)",
            page_path.display(),
            posts.len()
        );

        let archives = write_archive_hub(&cfg.out_dir).await?;
        println!(
            "saved {} (archive hub, {archives} archive{})",
            cfg.out_dir.join("index.html").display(),
            if archives == 1 { "" } else { "s" }
        );
    }

    Ok(Summary {
        posts: posts.len(),
        posts_new,
        media_total: report.total,
        media_downloaded: report.downloaded,
        media_cached: report.cached,
        media_failed: report.failed,
        media_converted: report.converted,
        media_resized: report.resized,
        media_skipped: skipped_small,
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

    fn post(id: &str, created: i64) -> Post {
        Post {
            id: id.into(),
            created_utc: created,
            ..Post::default()
        }
    }

    #[test]
    fn merge_keeps_fresh_data_and_appends_older_posts() {
        let fetched = vec![post("b", 20), post("c", 30)];
        let previous = vec![post("a", 10), post("b", 5), post("d", 40)];
        let merged = merge_posts(fetched, previous);
        let ids: Vec<&str> = merged.iter().map(|p| p.id.as_str()).collect();
        // fresh listing order first, then previously archived posts newest first
        assert_eq!(ids, vec!["b", "c", "d", "a"]);
        // the fresh entry wins over the stale copy
        assert_eq!(merged[0].created_utc, 20);
    }

    #[test]
    fn merge_drops_empty_and_duplicate_ids() {
        let merged = merge_posts(
            vec![post("", 1), post("a", 2), post("a", 3)],
            vec![post("a", 4), post("", 5), post("", 6)],
        );
        let ids: Vec<&str> = merged.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
        assert_eq!(merged[0].created_utc, 2);
    }

    #[tokio::test]
    async fn scan_archives_reads_metadata_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("r_funny");
        std::fs::create_dir_all(sub.join("media")).unwrap();
        std::fs::write(sub.join("media/subreddit_icon.png"), b"x").unwrap();
        std::fs::write(sub.join("index.html"), b"<html>").unwrap();
        std::fs::write(
            sub.join("funny_about.json"),
            json!({"about": {"name": "funny", "title": "F U N N Y", "icon": "https://i.example/i.png"}})
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            sub.join("funny_posts.json"),
            json!({
                "target": "r/funny",
                "fetched_at": "2026-09-29T00:00:00+00:00",
                "posts": [{"id": "a"}, {"id": "b"}],
            })
            .to_string(),
        )
        .unwrap();
        // directories that are not archives are ignored
        std::fs::create_dir_all(root.join("notes")).unwrap();

        let entries = scan_archives(root).await;
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.dir, "r_funny");
        assert_eq!(e.display, "r/funny");
        assert_eq!(e.title.as_deref(), Some("F U N N Y"));
        assert_eq!(e.icon.as_deref(), Some("r_funny/media/subreddit_icon.png"));
        assert_eq!(e.icon_remote.as_deref(), Some("https://i.example/i.png"));
        assert_eq!(e.posts, 2);
        assert!(e.viewer);
    }

    #[tokio::test]
    async fn scan_archives_falls_back_to_directory_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("u_spez")).unwrap();
        let entries = scan_archives(dir.path()).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display, "u/spez");
        assert_eq!(entries[0].posts, 0);
        assert!(!entries[0].viewer);
    }
}
