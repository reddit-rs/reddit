//! Archive Reddit listings as JSON and media, with an offline HTML viewer.
//!
//! Build a [`Config`] for a [`Target`] (see [`parse_target`] for names, paths
//! and URLs) and call [`run`]. Each run fetches the listing, merges it into
//! the archive saved by earlier runs, downloads missing media and optionally
//! renders the viewer.
//!
//! ```no_run
//! # async fn example() -> anyhow::Result<()> {
//! let parsed = reddit::parse_target("https://www.reddit.com/r/rust/top/?t=week")?;
//! let mut config = reddit::Config::new(parsed.target);
//! config.sort = parsed.sort.unwrap_or_default();
//! config.time = parsed.time.unwrap_or_default();
//! config.cookies = reddit::Cookies::from_arg("cookies.txt")?;
//! config.offline = true;
//! let summary = reddit::run(&config).await?;
//! println!("{} posts", summary.posts);
//! # Ok(())
//! # }
//! ```

mod archive;
mod client;
mod cookies;
mod download;
mod html;
mod http;
mod manifest;
mod models;
mod normalize;
mod pipeline;
mod target;
mod transform;

pub use archive::{AboutFile, PostsFile};
pub use client::{MAX_PAGES, PostsMode};
pub use cookies::Cookies;
pub use download::MediaReport;
pub use manifest::Selection;
pub use models::{
    AssetKind, ConvertFormat, GalleryItem, GalleryKind, ManifestItem, MediaAsset, MediaFormat,
    MediaKind, Post, Size, Sort, SubredditInfo, TimeFilter, VideoInfo,
};
pub use pipeline::{Config, Summary, run};
pub use target::{ParsedTarget, Target, TargetKind, parse_target};
pub use transform::TransformOptions;
