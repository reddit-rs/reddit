//! Decide which files to download for an archive.

use crate::models::{
    AssetKind, ConvertFormat, ManifestItem, MediaAsset, MediaFormat, Post, Size, SubredditInfo,
    VideoInfo,
};

/// Which media a run downloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Include videos (implied by selecting a video format).
    pub videos: bool,
    /// Only these formats; empty selects every format.
    pub formats: Vec<MediaFormat>,
    /// Maximum items per gallery; `None` keeps all.
    pub gallery_limit: Option<usize>,
    /// Download the subreddit icon and banner.
    pub subreddit_art: bool,
    /// Skip post stills whose short side is below `width` or long side below
    /// `height`, regardless of orientation.
    pub min_size: Option<Size>,
}

/// The files to download.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub items: Vec<ManifestItem>,
    /// Items dropped by [`Selection::min_size`].
    pub skipped_small: usize,
}

/// Build the manifest: subreddit art first, then each post's media in
/// listing order. Stills that `convert` will re-encode get its extension.
pub fn build(
    about: Option<&SubredditInfo>,
    posts: &[Post],
    selection: &Selection,
    convert: Option<ConvertFormat>,
) -> Manifest {
    let videos = selection.videos || selection.formats.iter().any(|f| f.is_video());
    let mut items = Vec::new();

    if selection.subreddit_art
        && let Some(about) = about
    {
        for (kind, url) in [
            (AssetKind::Icon, &about.icon),
            (AssetKind::Banner, &about.banner),
        ] {
            if let Some(url) = url {
                items.push(art(kind, url));
            }
        }
    }

    for post in posts {
        if post.gallery.is_empty() {
            if let Some(image) = post_image(post) {
                items.push(still(&post.id, AssetKind::Image, None, &image));
            }
        } else {
            let limit = selection.gallery_limit.unwrap_or(usize::MAX);
            for item in post.gallery.iter().take_while(|item| item.index < limit) {
                if let Some(image) = &item.image {
                    items.push(still(&post.id, AssetKind::Gallery, Some(item.index), image));
                }
                if videos && let Some(video) = &item.video {
                    items.push(self::video(&post.id, Some(item.index), video));
                }
            }
        }
        if videos && let Some(video) = &post.video {
            items.push(self::video(&post.id, None, video));
        }
    }

    // `--formats gif` must never download something else, so items of
    // unknown format are dropped when a selection is given.
    if !selection.formats.is_empty() {
        items.retain(|item| item.format.is_some_and(|f| selection.formats.contains(&f)));
    }
    let before = items.len();
    if let Some(min) = selection.min_size {
        items.retain(|item| large_enough(item, min));
    }
    let skipped_small = before - items.len();

    if let Some(target) = convert.map(MediaFormat::from) {
        for item in &mut items {
            if item.kind.is_post_still() && item.format.is_some_and(MediaFormat::is_static_image) {
                item.format = Some(target);
            }
        }
    }
    Manifest {
        items,
        skipped_small,
    }
}

/// Stored format of a URL: a resizer `format=` parameter wins (it is what the
/// server sends), then metadata, then the path extension.
fn detect_format(url: &str, known: Option<MediaFormat>) -> Option<MediaFormat> {
    MediaFormat::from_url_query(url)
        .or(known)
        .or_else(|| MediaFormat::from_url_path(url))
}

fn is_still_url(url: &str) -> bool {
    MediaFormat::from_url_path(url).is_some_and(|f| !f.is_video())
}

/// The image representing a non-gallery post: its direct image link (with
/// the preview as fallback), the preview, or the listing thumbnail.
fn post_image(post: &Post) -> Option<MediaAsset> {
    let direct = post.url.as_deref().filter(|u| is_still_url(u));
    match (direct, &post.preview) {
        (Some(url), preview) => Some(MediaAsset {
            url: url.to_owned(),
            fallback: preview.as_ref().map(|p| p.url.clone()),
            width: preview.as_ref().and_then(|p| p.width),
            height: preview.as_ref().and_then(|p| p.height),
            format: None,
        }),
        (None, Some(preview)) => Some(preview.clone()),
        (None, None) => post
            .thumbnail
            .as_deref()
            .filter(|u| is_still_url(u))
            .map(|url| MediaAsset {
                url: url.to_owned(),
                fallback: None,
                width: None,
                height: None,
                format: None,
            }),
    }
}

fn still(post_id: &str, kind: AssetKind, index: Option<usize>, image: &MediaAsset) -> ManifestItem {
    ManifestItem {
        post_id: post_id.to_owned(),
        kind,
        index,
        url: image.url.clone(),
        fallback: image.fallback.clone().filter(|f| *f != image.url),
        format: detect_format(&image.url, image.format),
        width: image.width,
        height: image.height,
    }
}

fn video(post_id: &str, index: Option<usize>, video: &VideoInfo) -> ManifestItem {
    ManifestItem {
        post_id: post_id.to_owned(),
        kind: AssetKind::Video,
        index,
        url: video.url.clone(),
        fallback: None,
        format: Some(
            MediaFormat::from_url_path(&video.url)
                .filter(|f| f.is_video())
                .unwrap_or(MediaFormat::Mp4),
        ),
        width: video.width,
        height: video.height,
    }
}

fn art(kind: AssetKind, url: &str) -> ManifestItem {
    ManifestItem {
        post_id: String::new(),
        kind,
        index: None,
        url: url.to_owned(),
        fallback: None,
        format: detect_format(url, None),
        width: None,
        height: None,
    }
}

/// Unknown dimensions, videos and subreddit art always pass.
fn large_enough(item: &ManifestItem, min: Size) -> bool {
    let (Some(width), Some(height)) = (item.width, item.height) else {
        return true;
    };
    if !item.kind.is_post_still() || width == 0 || height == 0 {
        return true;
    }
    width.min(height) >= min.width.get() && width.max(height) >= min.height.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::{self, tests::gallery_json};
    use serde_json::json;

    fn kinds(manifest: &Manifest) -> Vec<AssetKind> {
        manifest.items.iter().map(|i| i.kind).collect()
    }

    fn about() -> SubredditInfo {
        SubredditInfo {
            name: "testsub".into(),
            icon: Some("https://styles.redditmedia.com/icon.png?w=256".into()),
            banner: Some("https://styles.redditmedia.com/banner.jpg?w=1".into()),
            ..SubredditInfo::default()
        }
    }

    #[test]
    fn galleries_art_order_and_limits() {
        use AssetKind::*;
        let post = normalize::post(&gallery_json(), "https://i.redd.it");
        let all = Selection {
            subreddit_art: true,
            ..Selection::default()
        };
        let manifest = build(Some(&about()), std::slice::from_ref(&post), &all, None);
        assert_eq!(kinds(&manifest), [Icon, Banner, Gallery, Gallery, Gallery]);
        let formats: Vec<_> = manifest.items.iter().map(|i| i.format).collect();
        use MediaFormat::*;
        assert_eq!(
            formats,
            [Some(Png), Some(Jpg), Some(Jpg), Some(Png), Some(Jpg)]
        );
        assert_eq!(manifest.items[4].index, Some(2));

        let limited = Selection {
            gallery_limit: Some(2),
            videos: true,
            ..Selection::default()
        };
        let manifest = build(Some(&about()), std::slice::from_ref(&post), &limited, None);
        assert_eq!(kinds(&manifest), [Gallery, Gallery, Video]);
    }

    #[test]
    fn direct_images_fall_back_to_previews_and_videos_are_opt_in() {
        let post = normalize::post(
            &json!({
                "id": "img1",
                "url": "https://i.redd.it/img1.jpeg",
                "preview": {"images": [{"source": {"url": "https://preview.redd.it/img1.jpeg?s=1", "width": 1080, "height": 1080}}]},
                "media": {"reddit_video": {"fallback_url": "https://v.redd.it/x/DASH_720.mp4?source=fallback"}}
            }),
            "https://i.redd.it",
        );
        let manifest = build(
            None,
            std::slice::from_ref(&post),
            &Selection::default(),
            None,
        );
        assert_eq!(kinds(&manifest), [AssetKind::Image]);
        let image = &manifest.items[0];
        assert_eq!(image.url, "https://i.redd.it/img1.jpeg");
        assert_eq!(
            image.fallback.as_deref(),
            Some("https://preview.redd.it/img1.jpeg?s=1")
        );
        assert_eq!(image.format, Some(MediaFormat::Jpg));
        assert_eq!(image.width, Some(1080));

        let videos = Selection {
            videos: true,
            ..Selection::default()
        };
        let manifest = build(None, std::slice::from_ref(&post), &videos, None);
        assert_eq!(kinds(&manifest), [AssetKind::Image, AssetKind::Video]);
        assert_eq!(manifest.items[1].format, Some(MediaFormat::Mp4));
    }

    #[test]
    fn resizer_format_query_decides_the_format() {
        let post = normalize::post(
            &json!({"id": "ext1", "url": "https://external-preview.redd.it/abc.png?format=pjpg&auto=webp&s=1"}),
            "https://i.redd.it",
        );
        let manifest = build(None, &[post], &Selection::default(), None);
        assert_eq!(manifest.items[0].format, Some(MediaFormat::Jpg));
    }

    fn sized(
        id: &str,
        kind: AssetKind,
        format: MediaFormat,
        w: Option<u32>,
        h: Option<u32>,
    ) -> Post {
        let mut post = Post {
            id: id.into(),
            ..Post::default()
        };
        let asset = MediaAsset {
            url: format!("https://i.redd.it/{id}.{format}"),
            fallback: None,
            width: w,
            height: h,
            format: None,
        };
        match kind {
            AssetKind::Video => {
                post.video = Some(VideoInfo {
                    url: asset.url,
                    width: w,
                    height: h,
                    duration: None,
                })
            }
            _ => post.preview = Some(asset),
        }
        post
    }

    #[test]
    fn format_selection_and_min_size() {
        use MediaFormat::*;
        let posts = [
            sized(
                "portrait_floor",
                AssetKind::Image,
                Jpg,
                Some(768),
                Some(1024),
            ),
            sized(
                "portrait_narrow",
                AssetKind::Image,
                Jpg,
                Some(700),
                Some(1200),
            ),
            sized(
                "landscape_floor",
                AssetKind::Image,
                Png,
                Some(1024),
                Some(768),
            ),
            sized(
                "landscape_short",
                AssetKind::Image,
                Jpg,
                Some(1200),
                Some(700),
            ),
            sized("unknown", AssetKind::Image, Gif, None, None),
            sized("clip", AssetKind::Video, Mp4, Some(10), Some(10)),
        ];
        let ids = |m: &Manifest| {
            m.items
                .iter()
                .map(|i| i.post_id.clone())
                .collect::<Vec<_>>()
        };

        let min = Selection {
            min_size: Size::new(768, 1024),
            videos: true,
            subreddit_art: true,
            ..Selection::default()
        };
        let manifest = build(Some(&about()), &posts, &min, None);
        assert_eq!(
            ids(&manifest),
            [
                "",
                "",
                "portrait_floor",
                "landscape_floor",
                "unknown",
                "clip"
            ]
        );
        assert_eq!(manifest.skipped_small, 2);

        let gifs = Selection {
            formats: vec![Gif],
            ..Selection::default()
        };
        assert_eq!(ids(&build(None, &posts, &gifs, None)), ["unknown"]);

        let mp4 = Selection {
            formats: vec![Mp4],
            ..Selection::default()
        };
        assert_eq!(
            ids(&build(None, &posts, &mp4, None)),
            ["clip"],
            "video formats imply videos"
        );
    }

    #[test]
    fn conversion_rewrites_only_static_post_stills() {
        use MediaFormat::*;
        let posts = [
            sized("png", AssetKind::Image, Png, None, None),
            sized("jpg", AssetKind::Image, Jpg, None, None),
            sized("gif", AssetKind::Image, Gif, None, None),
            sized("webp", AssetKind::Image, Webp, None, None),
            sized("clip", AssetKind::Video, Webm, None, None),
        ];
        let all = Selection {
            videos: true,
            subreddit_art: true,
            ..Selection::default()
        };
        let manifest = build(Some(&about()), &posts, &all, Some(ConvertFormat::Jpg));
        let formats: Vec<_> = manifest.items.iter().map(|i| i.format.unwrap()).collect();
        // art keeps its format; png → jpg; gif/webp may be animated
        assert_eq!(formats, [Png, Jpg, Jpg, Jpg, Gif, Webp, Webm]);
    }
}
