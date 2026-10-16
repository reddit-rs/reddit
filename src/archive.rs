//! On-disk archive layout: typed JSON documents, atomic writes, incremental
//! merging and discovery of sibling archives for the hub page.
//!
//! ```text
//! <out_dir>/<r_|u_><name>/
//!   posts.json      PostsFile (merged across runs)
//!   about.json      AboutFile (single subreddits only)
//!   raw.json        latest raw listing
//!   manifest.json   selected media
//!   index.html      offline viewer
//!   media/          downloaded files
//! ```

use crate::models::{Post, Sort, SubredditInfo, TimeFilter};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const POSTS_FILE: &str = "posts.json";
pub const ABOUT_FILE: &str = "about.json";
pub const RAW_FILE: &str = "raw.json";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const INDEX_FILE: &str = "index.html";

/// `posts.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PostsFile {
    /// `r/name` or `u/name`.
    pub target: String,
    pub sort: Sort,
    pub time: TimeFilter,
    /// RFC 3339 time of the latest run.
    pub fetched_at: String,
    pub posts: Vec<Post>,
}

/// `about.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AboutFile {
    pub target: String,
    pub fetched_at: String,
    pub about: SubredditInfo,
}

pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// Replace `path` only after a `.part` sibling was fully written, so readers
/// and later runs never see truncated files. One writer per archive.
pub(crate) async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = part_path(path);
    let result = async {
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result.with_context(|| format!("cannot write {}", path.display()))
}

pub(crate) async fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?).await
}

/// `None` when the file is missing.
async fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .with_context(|| format!("cannot parse {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Posts saved by previous runs. An unreadable file is an error rather than
/// an empty archive, so a re-run can never silently discard history.
pub(crate) async fn load_posts(dir: &Path) -> Result<Vec<Post>> {
    let file: Option<PostsFile> = read_json(&dir.join(POSTS_FILE)).await?;
    Ok(file.map(|f| f.posts).unwrap_or_default())
}

/// Merge a fresh listing into the archive: fetched posts keep their (newer)
/// data and listing order; previously archived posts follow, newest first.
/// Duplicates and posts without an id are dropped.
pub(crate) fn merge_posts(fetched: Vec<Post>, previous: Vec<Post>) -> Vec<Post> {
    let mut seen = HashSet::new();
    let mut keep = |post: &Post| !post.id.is_empty() && seen.insert(post.id.clone());

    let mut merged: Vec<Post> = fetched.into_iter().filter(&mut keep).collect();
    let mut older: Vec<Post> = previous.into_iter().filter(&mut keep).collect();
    older.sort_by(|a, b| {
        b.created_utc
            .cmp(&a.created_utc)
            .then_with(|| a.id.cmp(&b.id))
    });
    merged.extend(older);
    merged
}

/// One archive directory shown on the hub page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Directory under the output root (`r_funny`).
    pub dir: String,
    /// Listing name (`r/funny`, `u/spez`).
    pub display: String,
    pub title: Option<String>,
    /// Local icon, relative to the output root.
    pub icon: Option<String>,
    /// Remote icon, used when the local file is missing.
    pub icon_remote: Option<String>,
    pub posts: usize,
    pub fetched_at: Option<String>,
    pub over_18: bool,
    /// Whether the archive has an `index.html` viewer.
    pub viewer: bool,
}

/// Describe every `r_*` / `u_*` archive under `root`, sorted by directory.
/// Unreadable archives are listed with whatever metadata is available.
pub(crate) async fn scan(root: &Path) -> Result<Vec<ArchiveEntry>> {
    let mut entries = Vec::new();
    let mut dir = tokio::fs::read_dir(root)
        .await
        .with_context(|| format!("cannot list {}", root.display()))?;
    while let Some(entry) = dir.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((prefix, rest)) = name.split_once('_') else {
            continue;
        };
        if !matches!(prefix, "r" | "u") || rest.is_empty() {
            continue;
        }

        let path = entry.path();
        let posts: Option<PostsFile> = read_json(&path.join(POSTS_FILE)).await.ok().flatten();
        let about: Option<AboutFile> = read_json(&path.join(ABOUT_FILE)).await.ok().flatten();
        let about = about.map(|a| a.about);
        entries.push(ArchiveEntry {
            display: posts
                .as_ref()
                .map_or_else(|| format!("{prefix}/{rest}"), |p| p.target.clone()),
            title: about.as_ref().and_then(|a| a.title.clone()),
            icon: local_icon(&path.join("media"))
                .await
                .map(|file| format!("{name}/media/{file}")),
            icon_remote: about.as_ref().and_then(|a| a.icon.clone()),
            posts: posts.as_ref().map_or(0, |p| p.posts.len()),
            fetched_at: posts.map(|p| p.fetched_at),
            over_18: about.is_some_and(|a| a.over_18),
            viewer: tokio::fs::try_exists(path.join(INDEX_FILE))
                .await
                .unwrap_or(false),
            dir: name,
        });
    }
    entries.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(entries)
}

/// File name of `media/subreddit_icon.*`, if downloaded.
async fn local_icon(media: &Path) -> Option<String> {
    let mut dir = tokio::fs::read_dir(media).await.ok()?;
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("subreddit_icon.") {
            return Some(name);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn post(id: &str, created: i64) -> Post {
        Post {
            id: id.into(),
            created_utc: created,
            ..Post::default()
        }
    }

    fn ids(posts: &[Post]) -> Vec<&str> {
        posts.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn merge_keeps_fresh_data_and_appends_older_posts() {
        let merged = merge_posts(
            vec![post("b", 20), post("c", 30)],
            vec![post("a", 10), post("b", 5), post("d", 40)],
        );
        assert_eq!(ids(&merged), ["b", "c", "d", "a"]);
        assert_eq!(merged[0].created_utc, 20, "the fresh copy wins");
    }

    #[test]
    fn merge_drops_empty_and_duplicate_ids() {
        let merged = merge_posts(
            vec![post("", 1), post("a", 2), post("a", 3)],
            vec![post("a", 4), post("", 5)],
        );
        assert_eq!(ids(&merged), ["a"]);
        assert_eq!(merged[0].created_utc, 2);
    }

    #[tokio::test]
    async fn atomic_writes_replace_files_and_clean_up_failures() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("posts.json");
        write_atomic(&file, b"old").await.unwrap();
        write_atomic(&file, b"new").await.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        assert!(!part_path(&file).exists());

        let directory = dir.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(write_atomic(&directory, b"data").await.is_err());
        assert!(directory.is_dir());
        assert!(!part_path(&directory).exists());
    }

    #[tokio::test]
    async fn corrupt_archives_are_errors_and_missing_ones_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_posts(dir.path()).await.unwrap().is_empty());
        std::fs::write(dir.path().join(POSTS_FILE), b"{not json").unwrap();
        assert!(load_posts(dir.path()).await.is_err());
    }

    #[tokio::test]
    async fn scan_reads_archive_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let funny = root.join("r_funny");
        std::fs::create_dir_all(funny.join("media")).unwrap();
        std::fs::write(funny.join("media/subreddit_icon.png"), b"x").unwrap();
        std::fs::write(funny.join(INDEX_FILE), b"<html>").unwrap();
        std::fs::write(
            funny.join(ABOUT_FILE),
            json!({
                "target": "r/funny",
                "fetched_at": "2026-09-29T00:00:00+00:00",
                "about": {"name": "funny", "title": "F U N N Y", "icon": "https://i.example/i.png"}
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            funny.join(POSTS_FILE),
            json!({
                "target": "r/funny",
                "sort": "hot",
                "time": "all",
                "fetched_at": "2026-09-29T00:00:00+00:00",
                "posts": [
                    {"id": "a", "title": "", "permalink": "", "created_utc": 0},
                    {"id": "b", "title": "", "permalink": "", "created_utc": 0}
                ],
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("u_spez")).unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("r_file"), b"not a directory").unwrap();

        let entries = scan(root).await.unwrap();
        assert_eq!(entries.len(), 2);
        let funny = &entries[0];
        assert_eq!(funny.dir, "r_funny");
        assert_eq!(funny.display, "r/funny");
        assert_eq!(funny.title.as_deref(), Some("F U N N Y"));
        assert_eq!(
            funny.icon.as_deref(),
            Some("r_funny/media/subreddit_icon.png")
        );
        assert_eq!(
            funny.icon_remote.as_deref(),
            Some("https://i.example/i.png")
        );
        assert_eq!(funny.posts, 2);
        assert!(funny.viewer);

        let spez = &entries[1];
        assert_eq!(spez.display, "u/spez");
        assert_eq!((spez.posts, spez.viewer), (0, false));
    }
}
