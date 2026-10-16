//! Raw Reddit JSON (`t3` posts, `t5` subreddits) → normalized models.

use crate::models::{
    GalleryItem, GalleryKind, MediaAsset, MediaFormat, Post, SubredditInfo, VideoInfo,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

fn non_empty(v: &Value, key: &str) -> Option<String> {
    string(v, key).filter(|s| !s.is_empty())
}

/// Reddit serializes counts as integers but timestamps as floats.
fn int(v: &Value, key: &str) -> Option<i64> {
    let x = v.get(key)?;
    x.as_i64().or_else(|| x.as_f64().map(|f| f as i64))
}

fn float(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64()
}

fn dimension(v: &Value, key: &str) -> Option<u32> {
    int(v, key).and_then(|n| u32::try_from(n).ok())
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `raw_json=1` already decodes entities; URLs fetched without it do not.
fn url(v: &Value, key: &str) -> Option<String> {
    string(v, key).map(|u| u.replace("&amp;", "&"))
}

fn rfc3339(ts: i64) -> Option<String> {
    (ts != 0)
        .then(|| DateTime::<Utc>::from_timestamp(ts, 0))
        .flatten()
        .map(|d| d.to_rfc3339())
}

/// Creation time of a raw post, in Unix seconds.
pub(crate) fn created_utc(d: &Value) -> i64 {
    int(d, "created_utc").unwrap_or(0)
}

/// Field of `d`, falling back to the crosspost parent: crossposts often carry
/// their media only on the original post.
fn field<'a>(d: &'a Value, parent: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    d.get(key).or_else(|| parent?.get(key))
}

/// Highest-resolution preview image (`preview.images[0].source`).
fn preview_image(preview: Option<&Value>) -> Option<MediaAsset> {
    let source = preview?.pointer("/images/0/source")?;
    Some(MediaAsset {
        url: url(source, "url")?,
        fallback: None,
        width: dimension(source, "width"),
        height: dimension(source, "height"),
        format: None,
    })
}

/// Reddit-hosted video (`media.reddit_video`).
fn reddit_video(media: Option<&Value>) -> Option<VideoInfo> {
    let video = media?.get("reddit_video")?;
    Some(VideoInfo {
        url: url(video, "fallback_url")?,
        width: dimension(video, "width"),
        height: dimension(video, "height"),
        duration: float(video, "duration"),
    })
}

/// Gallery items in display order. Each image prefers the original
/// `<media_base>/<media_id>.<ext>` file and keeps the signed preview as a
/// fallback.
fn gallery(d: &Value, parent: Option<&Value>, media_base: &str) -> Vec<GalleryItem> {
    let Some(items) = field(d, parent, "gallery_data")
        .and_then(|g| g.get("items"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let metadata = field(d, parent, "media_metadata");

    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let media_id = string(item, "media_id")?;
            let meta = metadata.and_then(|m| m.get(&media_id));
            let source = meta.and_then(|m| m.get("s"));
            let mime = meta.and_then(|m| string(m, "m")).unwrap_or_default();
            let format = MediaFormat::from_mime(&mime);
            let width = source.and_then(|s| dimension(s, "x"));
            let height = source.and_then(|s| dimension(s, "y"));
            let still = source.and_then(|s| url(s, "u"));
            let mp4 = source.and_then(|s| url(s, "mp4"));

            let is_video = mime.starts_with("video/")
                || meta.and_then(|m| m.get("e")).and_then(Value::as_str) == Some("Video");
            let kind = match (is_video, mp4.is_some()) {
                (true, _) => GalleryKind::Video,
                (false, true) => GalleryKind::Animated,
                (false, false) => GalleryKind::Image,
            };

            // Only stills have a derivable original; video items keep their
            // preview frame as the image.
            let still_format = format.filter(|f| !f.is_video());
            let original = still_format
                .map(|f| format!("{}/{media_id}.{f}", media_base.trim_end_matches('/')));
            let image = match (original, still) {
                (Some(url), fallback) => Some(MediaAsset {
                    url,
                    fallback,
                    width,
                    height,
                    format: still_format,
                }),
                (None, Some(url)) => Some(MediaAsset {
                    url,
                    fallback: None,
                    width,
                    height,
                    format: None,
                }),
                (None, None) => None,
            };
            let video = mp4.map(|url| VideoInfo {
                url,
                width,
                height,
                duration: None,
            });
            (image.is_some() || video.is_some()).then(|| GalleryItem {
                index,
                media_id,
                kind,
                image,
                video,
                caption: non_empty(item, "caption"),
            })
        })
        .collect()
}

/// Normalize one raw listing item (the `data` of a `t3` child).
pub fn post(d: &Value, media_base: &str) -> Post {
    let parent = d.pointer("/crosspost_parent_list/0");
    let created = created_utc(d);
    Post {
        id: string(d, "id").unwrap_or_default(),
        name: string(d, "name"),
        title: string(d, "title").unwrap_or_default(),
        author: string(d, "author").unwrap_or_default(),
        subreddit: string(d, "subreddit").unwrap_or_default(),
        permalink: string(d, "permalink")
            .map(|p| format!("https://www.reddit.com{p}"))
            .unwrap_or_default(),
        url: url(d, "url"),
        domain: string(d, "domain"),
        post_hint: string(d, "post_hint"),
        created_utc: created,
        datetime: rfc3339(created),
        selftext: non_empty(d, "selftext"),
        score: int(d, "score"),
        upvote_ratio: float(d, "upvote_ratio"),
        num_comments: int(d, "num_comments"),
        num_crossposts: int(d, "num_crossposts"),
        over_18: flag(d, "over_18"),
        spoiler: flag(d, "spoiler"),
        stickied: flag(d, "stickied"),
        locked: flag(d, "locked"),
        is_self: flag(d, "is_self"),
        is_video: flag(d, "is_video"),
        is_gallery: flag(d, "is_gallery"),
        link_flair_text: non_empty(d, "link_flair_text"),
        author_flair_text: non_empty(d, "author_flair_text"),
        // Placeholders such as `self`, `nsfw` or `default` are not URLs.
        thumbnail: string(d, "thumbnail").filter(|t| t.starts_with("http")),
        preview: preview_image(field(d, parent, "preview")),
        video: reddit_video(field(d, parent, "media"))
            .or_else(|| reddit_video(field(d, parent, "secure_media"))),
        gallery: gallery(d, parent, media_base),
    }
}

/// Normalize `about.json` (`{kind: "t5", data: {...}}`).
pub fn subreddit(v: &Value) -> SubredditInfo {
    let d = v.get("data").unwrap_or(v);
    let created = int(d, "created_utc").filter(|&t| t != 0);
    let first_url = |keys: [&str; 2]| {
        keys.into_iter()
            .filter_map(|key| url(d, key))
            .find(|u| u.starts_with("http"))
    };
    SubredditInfo {
        name: string(d, "display_name").unwrap_or_default(),
        title: non_empty(d, "title"),
        description: non_empty(d, "public_description"),
        subscribers: int(d, "subscribers"),
        created_utc: created,
        datetime: created.and_then(rfc3339),
        over_18: flag(d, "over18"),
        icon: first_url(["community_icon", "icon_img"]),
        banner: first_url(["banner_background_image", "banner_img"]),
        primary_color: non_empty(d, "primary_color"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn gallery_json() -> Value {
        json!({
            "id": "abc123",
            "name": "t3_abc123",
            "title": "A gallery",
            "author": "someone",
            "subreddit": "testsub",
            "permalink": "/r/testsub/comments/abc123/a_gallery/",
            "url": "https://www.reddit.com/gallery/abc123",
            "created_utc": 1700100000.5,
            "score": 42,
            "upvote_ratio": 0.95,
            "num_comments": 3,
            "over_18": true,
            "is_gallery": true,
            "link_flair_text": "Pics",
            "thumbnail": "nsfw",
            "gallery_data": {"items": [
                {"media_id": "m0", "id": 1},
                {"media_id": "m1", "caption": "second", "id": 2},
                {"media_id": "m2", "id": 3}
            ]},
            "media_metadata": {
                "m0": {"status": "valid", "e": "Image", "m": "image/jpg",
                       "s": {"u": "https://preview.redd.it/m0.jpg?width=1000&amp;format=pjpg", "x": 1000, "y": 2000}},
                "m1": {"status": "valid", "e": "AnimatedImage", "m": "image/png",
                       "s": {"u": "https://preview.redd.it/m1.png?width=500", "mp4": "https://preview.redd.it/m1.mp4", "x": 500, "y": 500}},
                "m2": {"status": "valid", "e": "Image", "m": "video/mp4",
                       "s": {"u": "https://preview.redd.it/m2.jpg?width=300", "mp4": "https://preview.redd.it/m2.mp4", "x": 300, "y": 300}}
            }
        })
    }

    #[test]
    fn galleries_keep_order_originals_and_fallbacks() {
        let post = post(&gallery_json(), "https://i.redd.it");
        assert_eq!(post.id, "abc123");
        assert_eq!(post.created_utc, 1700100000);
        assert_eq!(post.thumbnail, None);
        assert_eq!(post.gallery.len(), 3);

        let first = post.gallery[0].image.as_ref().unwrap();
        assert_eq!(post.gallery[0].kind, GalleryKind::Image);
        assert_eq!(first.url, "https://i.redd.it/m0.jpg");
        assert_eq!(first.format, Some(MediaFormat::Jpg));
        assert_eq!(
            first.fallback.as_deref(),
            Some("https://preview.redd.it/m0.jpg?width=1000&format=pjpg")
        );
        assert_eq!(post.gallery[1].kind, GalleryKind::Animated);
        assert_eq!(post.gallery[1].caption.as_deref(), Some("second"));
        assert_eq!(post.gallery[2].kind, GalleryKind::Video);
        assert!(post.gallery[2].video.is_some());
        assert_eq!(
            post.gallery[2].image.as_ref().unwrap().url,
            "https://preview.redd.it/m2.jpg?width=300",
            "video items use their still preview, not a derived .mp4"
        );
    }

    #[test]
    fn galleries_use_the_preview_when_no_original_is_derivable() {
        let mut v = gallery_json();
        v["media_metadata"]["m0"]["m"] = json!("application/octet-stream");
        let post = post(&v, "https://i.redd.it");
        let image = post.gallery[0].image.as_ref().unwrap();
        assert_eq!(
            image.url,
            "https://preview.redd.it/m0.jpg?width=1000&format=pjpg"
        );
        assert!(image.fallback.is_none());
    }

    #[test]
    fn crossposts_use_parent_media() {
        let v = json!({
            "id": "cross1",
            "title": "crosspost",
            "is_gallery": true,
            "crosspost_parent_list": [gallery_json()],
        });
        let post = post(&v, "https://i.redd.it");
        assert_eq!(post.gallery.len(), 3);
        assert!(post.preview.is_none());
    }

    #[test]
    fn previews_and_videos() {
        let v = json!({
            "id": "img1",
            "title": "direct image",
            "url": "https://i.redd.it/img1.jpeg",
            "thumbnail": "https://b.thumbs.redditmedia.com/t.jpg",
            "preview": {"images": [{"source": {"url": "https://preview.redd.it/img1.jpeg?s=1", "width": 1080, "height": 1080}}]},
            "is_video": true,
            "media": {"reddit_video": {"fallback_url": "https://v.redd.it/img1/DASH_720.mp4?source=fallback", "width": 720, "height": 1280, "duration": 12.0}}
        });
        let post = post(&v, "https://i.redd.it");
        assert_eq!(post.media_kind(), crate::models::MediaKind::Video);
        assert_eq!(post.preview.as_ref().unwrap().width, Some(1080));
        let video = post.video.as_ref().unwrap();
        assert_eq!((video.width, video.duration), (Some(720), Some(12.0)));
    }

    #[test]
    fn subreddit_metadata() {
        let about = subreddit(&json!({
            "kind": "t5",
            "data": {
                "display_name": "testsub",
                "title": "Test",
                "public_description": "hello",
                "subscribers": 1000,
                "created_utc": 1600000000.0,
                "over18": true,
                "community_icon": "",
                "icon_img": "https://styles.redditmedia.com/icon.png?w=256&amp;s=1",
                "banner_background_image": "https://styles.redditmedia.com/banner.jpg"
            }
        }));
        assert_eq!(about.name, "testsub");
        assert_eq!(about.subscribers, Some(1000));
        assert!(about.over_18);
        assert_eq!(
            about.icon.as_deref(),
            Some("https://styles.redditmedia.com/icon.png?w=256&s=1")
        );
        assert_eq!(about.datetime.as_deref(), Some("2020-09-13T12:26:40+00:00"));
    }
}
