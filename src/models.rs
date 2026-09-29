use serde::{Deserialize, Serialize};

/// A media container the archive can store. Used by `--formats` to select
/// which files are downloaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MediaFormat {
    Jpg,
    Png,
    Gif,
    Webp,
    Bmp,
    Mp4,
    Webm,
}

impl MediaFormat {
    pub const ALL: [MediaFormat; 7] = [
        MediaFormat::Jpg,
        MediaFormat::Png,
        MediaFormat::Gif,
        MediaFormat::Webp,
        MediaFormat::Bmp,
        MediaFormat::Mp4,
        MediaFormat::Webm,
    ];

    /// Canonical file extension (`jpeg` is reported as `jpg`).
    pub fn name(self) -> &'static str {
        match self {
            MediaFormat::Jpg => "jpg",
            MediaFormat::Png => "png",
            MediaFormat::Gif => "gif",
            MediaFormat::Webp => "webp",
            MediaFormat::Bmp => "bmp",
            MediaFormat::Mp4 => "mp4",
            MediaFormat::Webm => "webm",
        }
    }

    /// Parse a file extension or mime alias. `jpeg` and `jpg` are the same
    /// format; matching is case-insensitive.
    pub fn from_ext(ext: &str) -> Option<Self> {
        Some(match ext.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "pjpg" => MediaFormat::Jpg,
            "png" => MediaFormat::Png,
            "gif" => MediaFormat::Gif,
            "webp" => MediaFormat::Webp,
            "bmp" => MediaFormat::Bmp,
            "mp4" => MediaFormat::Mp4,
            "webm" => MediaFormat::Webm,
            _ => return None,
        })
    }

    /// Video containers are only downloaded with `--videos` (or when
    /// explicitly requested through `--formats`).
    pub fn is_video(self) -> bool {
        matches!(self, MediaFormat::Mp4 | MediaFormat::Webm)
    }
}

/// What a CLI target points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// `r/<name>` — a single subreddit or a `+`-joined multi (e.g. `rust+golang`).
    Subreddit,
    /// `u/<name>` / `user/<name>` — a user's submitted posts.
    User,
}

impl TargetKind {
    pub fn name(self) -> &'static str {
        match self {
            TargetKind::Subreddit => "subreddit",
            TargetKind::User => "user",
        }
    }

    /// URL path prefix used by reddit (`/r/...` or `/user/...`).
    pub fn path_prefix(self) -> &'static str {
        match self {
            TargetKind::Subreddit => "r",
            TargetKind::User => "user",
        }
    }

    /// Prefix for the output directory (`output/r_rust`, `output/u_spez`).
    pub fn output_prefix(self) -> &'static str {
        match self {
            TargetKind::Subreddit => "r_",
            TargetKind::User => "u_",
        }
    }
}

/// A validated listing target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: TargetKind,
    /// Subreddit name (may contain `+` for multis) or username.
    pub name: String,
}

impl Target {
    pub fn subreddit(name: impl Into<String>) -> Self {
        Self {
            kind: TargetKind::Subreddit,
            name: name.into(),
        }
    }

    pub fn user(name: impl Into<String>) -> Self {
        Self {
            kind: TargetKind::User,
            name: name.into(),
        }
    }

    /// True for `r/a+b` style multi-subreddit targets.
    pub fn is_multi(&self) -> bool {
        self.kind == TargetKind::Subreddit && self.name.contains('+')
    }

    /// `r/rust` or `u/spez`.
    pub fn display(&self) -> String {
        format!("{}/{}", self.kind.path_prefix(), self.name)
    }

    /// Directory name under the output root (`r_rust`, `u_spez`).
    pub fn output_name(&self) -> String {
        format!("{}{}", self.kind.output_prefix(), self.name)
    }

    /// Canonical reddit URL of the listing.
    pub fn url(&self) -> String {
        format!("https://www.reddit.com/{}/", self.display())
    }
}

/// Listing sort order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Hot,
    New,
    Top,
    Rising,
    Controversial,
}

impl Sort {
    pub const ALL: [Sort; 5] = [
        Sort::Hot,
        Sort::New,
        Sort::Top,
        Sort::Rising,
        Sort::Controversial,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Sort::Hot => "hot",
            Sort::New => "new",
            Sort::Top => "top",
            Sort::Rising => "rising",
            Sort::Controversial => "controversial",
        }
    }

    pub fn from_name(s: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|sort| sort.name() == s)
    }

    /// Only `top` and `controversial` accept a time window.
    pub fn uses_time(self) -> bool {
        matches!(self, Sort::Top | Sort::Controversial)
    }
}

/// Time window for [`Sort::Top`] / [`Sort::Controversial`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeFilter {
    Hour,
    Day,
    Week,
    Month,
    Year,
    #[default]
    All,
}

impl TimeFilter {
    pub const ALL: [TimeFilter; 6] = [
        TimeFilter::Hour,
        TimeFilter::Day,
        TimeFilter::Week,
        TimeFilter::Month,
        TimeFilter::Year,
        TimeFilter::All,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TimeFilter::Hour => "hour",
            TimeFilter::Day => "day",
            TimeFilter::Week => "week",
            TimeFilter::Month => "month",
            TimeFilter::Year => "year",
            TimeFilter::All => "all",
        }
    }

    pub fn from_name(s: &str) -> Option<TimeFilter> {
        TimeFilter::ALL.into_iter().find(|time| time.name() == s)
    }
}

/// A downloadable image (or still frame of an animated item).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MediaAsset {
    /// Preferred URL (original `i.redd.it` file when derivable).
    pub url: String,
    /// Original/preview URL used when [`MediaAsset::url`] fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

/// A reddit-hosted video (`v.redd.it`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// One image of a gallery post, in display order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GalleryItem {
    /// Position in the gallery (`gallery_data.items`), used for file names.
    pub index: usize,
    pub media_id: String,
    /// `image`, `gif` (animated still) or `video`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<MediaAsset>,
    /// Video file of an animated item (`--videos`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// One cleaned reddit post.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Post {
    pub id: String,
    /// Full name (`t3_<id>`), if provided by the API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subreddit: String,
    /// Canonical reddit URL of the post.
    pub permalink: String,
    /// Outbound URL (the link target; equals the permalink for self posts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Reddit's `post_hint` (`image`, `rich:video`, `link`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_hint: Option<String>,
    pub created_utc: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selftext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upvote_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_comments: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_crossposts: Option<i64>,
    #[serde(default)]
    pub over_18: bool,
    #[serde(default)]
    pub spoiler: bool,
    #[serde(default)]
    pub stickied: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub is_self: bool,
    #[serde(default)]
    pub is_video: bool,
    #[serde(default)]
    pub is_gallery: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_flair_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_flair_text: Option<String>,
    /// Low-resolution thumbnail as provided by the listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// Highest-resolution preview image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<MediaAsset>,
    /// Reddit-hosted video, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gallery: Vec<GalleryItem>,
}

impl Post {
    /// Media kind used by the offline viewer and the manifest builder.
    pub fn media_kind(&self) -> &'static str {
        if !self.gallery.is_empty() {
            "gallery"
        } else if self.video.is_some() {
            "video"
        } else if self.preview.is_some() {
            "image"
        } else {
            "link"
        }
    }
}

/// Cleaned subreddit metadata (`/about.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SubredditInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribers: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_utc: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    #[serde(default)]
    pub over_18: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_color: Option<String>,
}

/// One entry of the download manifest.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ManifestItem {
    /// Subdirectory under `media/` (`posts`, or empty for subreddit art).
    pub folder: String,
    /// Post id, or `subreddit` for subreddit art.
    pub id: String,
    /// `image`, `gallery`, `thumb`, `cover`, `video`, `icon`, `banner`.
    pub kind: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
}
