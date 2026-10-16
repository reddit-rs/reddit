//! Domain types shared by the pipeline, the archive files and the viewer.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

/// A media container the archive can store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum MediaFormat {
    /// JPEG (`jpeg` is accepted too)
    #[cfg_attr(feature = "cli", value(alias = "jpeg"))]
    Jpg,
    Png,
    Gif,
    Webp,
    Bmp,
    /// MP4 video (implies --videos)
    Mp4,
    /// WebM video (implies --videos)
    Webm,
}

impl MediaFormat {
    /// Canonical file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
        }
    }

    /// Parse a file extension or Reddit resizer `format=` value
    /// (case-insensitive; `jpeg` and `pjpg` are JPEG).
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "pjpg" => Self::Jpg,
            "png" => Self::Png,
            "gif" => Self::Gif,
            "webp" => Self::Webp,
            "bmp" => Self::Bmp,
            "mp4" => Self::Mp4,
            "webm" => Self::Webm,
            _ => return None,
        })
    }

    /// Parse a MIME type; parameters such as `; charset=…` are ignored.
    pub fn from_mime(mime: &str) -> Option<Self> {
        let essence = mime.split(';').next().unwrap_or(mime).trim();
        let (top, sub) = essence.split_once('/')?;
        match top.to_ascii_lowercase().as_str() {
            "image" | "video" => Self::from_extension(sub),
            _ => None,
        }
    }

    /// Format of a URL's path extension (`?query` and `#fragment` ignored).
    pub fn from_url_path(url: &str) -> Option<Self> {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let file = path.rsplit('/').next().unwrap_or_default();
        Self::from_extension(file.rsplit_once('.')?.1)
    }

    /// Format requested through Reddit's image resizer (`format=pjpg`). This is
    /// what the server sends and may differ from the path extension.
    pub fn from_url_query(url: &str) -> Option<Self> {
        let query = url.split_once('?')?.1.split('#').next()?;
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| key.eq_ignore_ascii_case("format"))
            .and_then(|(_, value)| Self::from_extension(value))
    }

    /// Video containers are downloaded only with `--videos` or an explicit
    /// video format selection.
    pub fn is_video(self) -> bool {
        matches!(self, Self::Mp4 | Self::Webm)
    }

    /// Formats that are always still images. GIF and WebP may be animated, so
    /// they are never decoded, converted or resized.
    pub fn is_static_image(self) -> bool {
        matches!(self, Self::Jpg | Self::Png | Self::Bmp)
    }
}

impl fmt::Display for MediaFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// Target format for `--convert`. GIF/WebP are excluded because they may be
/// animated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum ConvertFormat {
    /// JPEG (`jpeg` is accepted too; transparency is flattened onto white)
    #[cfg_attr(feature = "cli", value(alias = "jpeg"))]
    Jpg,
    Png,
}

impl From<ConvertFormat> for MediaFormat {
    fn from(format: ConvertFormat) -> Self {
        match format {
            ConvertFormat::Jpg => Self::Jpg,
            ConvertFormat::Png => Self::Png,
        }
    }
}

/// A `width x height` box with both sides greater than zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: NonZeroU32,
    pub height: NonZeroU32,
}

impl Size {
    /// `None` when either side is zero.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Some(Self {
            width: NonZeroU32::new(width)?,
            height: NonZeroU32::new(height)?,
        })
    }
}

impl FromStr for Size {
    type Err = String;

    /// Parse `WxH` (`768x1024`, case-insensitive separator).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("invalid size '{s}': expected WxH with sides > 0, e.g. 768x1024");
        let (width, height) = s.trim().split_once(['x', 'X']).ok_or_else(invalid)?;
        let width = width.trim().parse().map_err(|_| invalid())?;
        let height = height.trim().parse().map_err(|_| invalid())?;
        Self::new(width, height).ok_or_else(invalid)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// Listing sort order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Hot,
    New,
    Top,
    Rising,
    Controversial,
}

impl Sort {
    const ALL: [Self; 5] = [
        Self::Hot,
        Self::New,
        Self::Top,
        Self::Rising,
        Self::Controversial,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::New => "new",
            Self::Top => "top",
            Self::Rising => "rising",
            Self::Controversial => "controversial",
        }
    }

    /// Case-insensitive lookup by name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|sort| sort.as_str().eq_ignore_ascii_case(name))
    }

    /// Only `top` and `controversial` accept a time window.
    pub fn uses_time(self) -> bool {
        matches!(self, Self::Top | Self::Controversial)
    }
}

/// Time window for [`Sort::Top`] and [`Sort::Controversial`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
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
    const ALL: [Self; 6] = [
        Self::Hour,
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Year,
        Self::All,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::All => "all",
        }
    }

    /// Case-insensitive lookup by name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|time| time.as_str().eq_ignore_ascii_case(name))
    }
}

/// A downloadable still image.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MediaAsset {
    /// Preferred URL (the original `i.redd.it` file when derivable).
    pub url: String,
    /// Signed preview URL tried when [`MediaAsset::url`] fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Format reported by Reddit's media metadata, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<MediaFormat>,
}

/// A video file (`v.redd.it` or a gallery animation).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// What a gallery item shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GalleryKind {
    Image,
    /// An animated image with an MP4 rendition.
    Animated,
    Video,
}

/// One gallery item, in display order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GalleryItem {
    /// Position in `gallery_data.items`; used for file names.
    pub index: usize,
    pub media_id: String,
    pub kind: GalleryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<MediaAsset>,
    /// Video rendition of animated items (downloaded with `--videos`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// The primary media of a post, as presented by the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Gallery,
    Video,
    Image,
    Link,
}

/// One normalized Reddit post.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Post {
    pub id: String,
    /// Full name (`t3_<id>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subreddit: String,
    /// Canonical Reddit URL of the post.
    pub permalink: String,
    /// Link target; equals the permalink for self posts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Reddit's `post_hint` (`image`, `rich:video`, `link`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_hint: Option<String>,
    /// Unix timestamp (seconds).
    pub created_utc: i64,
    /// RFC 3339 rendering of [`Post::created_utc`].
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
    /// Low-resolution listing thumbnail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// Highest-resolution preview image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<MediaAsset>,
    /// Reddit-hosted video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gallery: Vec<GalleryItem>,
}

impl Post {
    pub fn media_kind(&self) -> MediaKind {
        if !self.gallery.is_empty() {
            MediaKind::Gallery
        } else if self.video.is_some() {
            MediaKind::Video
        } else if self.preview.is_some() {
            MediaKind::Image
        } else {
            MediaKind::Link
        }
    }
}

/// Normalized subreddit metadata (`/about.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
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

/// The role of a manifest entry; decides its file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    /// Subreddit icon (`media/subreddit_icon.*`).
    Icon,
    /// Subreddit banner (`media/subreddit_banner.*`).
    Banner,
    /// Still image of a non-gallery post (`media/posts/<id>.*`).
    Image,
    /// Gallery item (`media/posts/<id>_<index>.*`).
    Gallery,
    /// Video (`media/posts/<id>.*` or `<id>_<index>.*` for gallery items).
    Video,
}

impl AssetKind {
    /// Post stills are the only items that size filters and transforms touch.
    pub fn is_post_still(self) -> bool {
        matches!(self, Self::Image | Self::Gallery)
    }
}

/// One file to download.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManifestItem {
    /// Post id; empty for subreddit art.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub post_id: String,
    pub kind: AssetKind,
    /// Gallery position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Stored format; `None` when it cannot be determined before downloading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<MediaFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl ManifestItem {
    /// Path relative to the archive directory, e.g. `media/posts/abc_00.jpg`.
    /// Unknown formats are stored as `.jpg` (`.mp4` for videos).
    pub fn path(&self) -> PathBuf {
        let ext = self
            .format
            .unwrap_or(match self.kind {
                AssetKind::Video => MediaFormat::Mp4,
                _ => MediaFormat::Jpg,
            })
            .extension();
        let media = PathBuf::from("media");
        match (self.kind, self.index) {
            (AssetKind::Icon, _) => media.join(format!("subreddit_icon.{ext}")),
            (AssetKind::Banner, _) => media.join(format!("subreddit_banner.{ext}")),
            (_, Some(index)) => media
                .join("posts")
                .join(format!("{}_{index:02}.{ext}", self.post_id)),
            (_, None) => media.join("posts").join(format!("{}.{ext}", self.post_id)),
        }
    }

    /// [`ManifestItem::path`] with forward slashes, for HTML and JSON.
    pub fn web_path(&self) -> String {
        self.path().to_string_lossy().replace('\\', "/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_formats_from_extensions_mimes_and_urls() {
        assert_eq!(MediaFormat::from_extension("JPEG"), Some(MediaFormat::Jpg));
        assert_eq!(MediaFormat::from_extension("pjpg"), Some(MediaFormat::Jpg));
        assert_eq!(MediaFormat::from_extension("gifv"), None);
        assert_eq!(
            MediaFormat::from_mime("image/jpeg; charset=utf-8"),
            Some(MediaFormat::Jpg)
        );
        assert_eq!(MediaFormat::from_mime("video/mp4"), Some(MediaFormat::Mp4));
        assert_eq!(MediaFormat::from_mime("application/octet-stream"), None);
        assert_eq!(MediaFormat::from_mime("text/png"), None);
        assert_eq!(
            MediaFormat::from_url_path("https://i.redd.it/a.GIF?s=1&format=pjpg"),
            Some(MediaFormat::Gif)
        );
        assert_eq!(
            MediaFormat::from_url_path("https://example.com/noext"),
            None
        );
        assert_eq!(
            MediaFormat::from_url_query("https://x.redd.it/a.png?width=1&format=pjpg&s=1"),
            Some(MediaFormat::Jpg)
        );
        assert_eq!(MediaFormat::from_url_query("https://i.redd.it/a.png"), None);
        assert_eq!(
            MediaFormat::from_url_query("https://i.redd.it/a.png?width=100"),
            None
        );
    }

    #[test]
    fn sizes_require_two_positive_dimensions() {
        assert_eq!("768x1024".parse(), Ok(Size::new(768, 1024).unwrap()));
        assert_eq!(" 768X1024 ".parse(), Ok(Size::new(768, 1024).unwrap()));
        for value in [
            "",
            "768",
            "0x1024",
            "768x0",
            "1x2x3",
            "-1x2",
            "4294967296x1",
        ] {
            assert!(value.parse::<Size>().is_err(), "{value}");
        }
        assert_eq!(Size::new(3, 4).unwrap().to_string(), "3x4");
    }

    #[test]
    fn sort_and_time_names_round_trip() {
        assert_eq!(Sort::from_name("TOP"), Some(Sort::Top));
        assert_eq!(Sort::from_name("best"), None);
        assert_eq!(TimeFilter::from_name("week"), Some(TimeFilter::Week));
        assert!(Sort::Controversial.uses_time() && !Sort::Hot.uses_time());
    }

    fn item(kind: AssetKind, index: Option<usize>, format: Option<MediaFormat>) -> ManifestItem {
        ManifestItem {
            post_id: if matches!(kind, AssetKind::Icon | AssetKind::Banner) {
                String::new()
            } else {
                "abc".into()
            },
            kind,
            index,
            url: String::new(),
            fallback: None,
            format,
            width: None,
            height: None,
        }
    }

    #[test]
    fn manifest_paths_cover_every_kind() {
        use MediaFormat::*;
        for (item, expected) in [
            (
                item(AssetKind::Image, None, Some(Jpg)),
                "media/posts/abc.jpg",
            ),
            (
                item(AssetKind::Gallery, Some(3), Some(Png)),
                "media/posts/abc_03.png",
            ),
            (
                item(AssetKind::Video, Some(1), Some(Mp4)),
                "media/posts/abc_01.mp4",
            ),
            (
                item(AssetKind::Video, None, Some(Webm)),
                "media/posts/abc.webm",
            ),
            (
                item(AssetKind::Icon, None, Some(Png)),
                "media/subreddit_icon.png",
            ),
            (
                item(AssetKind::Banner, None, Some(Jpg)),
                "media/subreddit_banner.jpg",
            ),
            (item(AssetKind::Image, None, None), "media/posts/abc.jpg"),
            (item(AssetKind::Video, None, None), "media/posts/abc.mp4"),
        ] {
            assert_eq!(item.web_path(), expected);
        }
    }
}
