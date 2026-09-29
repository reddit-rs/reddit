//! Archive Reddit listings as JSON and media, with an optional offline viewer.
//!
//! [`run`] fetches a listing, merges it with previously saved posts, and downloads
//! missing media. Configure the pipeline with [`Config`]; use [`parse_target`]
//! to accept subreddit names, user paths, or Reddit URLs.

pub mod clean;
pub mod client;
pub mod download;
pub mod html;
pub mod models;
pub mod transform;

mod pipeline;
mod storage;
mod target;

pub use download::{DownloadReport, TransformOptions};
pub use models::{
    GalleryItem, ManifestItem, MediaAsset, MediaFormat, Post, Sort, SubredditInfo, Target,
    TargetKind, TimeFilter, VideoInfo,
};
pub use pipeline::{Config, PostsMode, Summary, UA_DEFAULT, run};
pub use target::{ParsedTarget, parse_target};
