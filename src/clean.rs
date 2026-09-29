//! Raw reddit JSON → clean models, plus parsing helpers (cookies, dates, URLs).

use crate::models::*;
use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub fn get_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Reddit serializes counts as integers and timestamps as floats, so both
/// representations must be accepted.
pub fn get_i64(v: &Value, key: &str) -> Option<i64> {
    let x = v.get(key)?;
    x.as_i64().or_else(|| x.as_f64().map(|f| f as i64))
}

pub fn get_f64(v: &Value, key: &str) -> Option<f64> {
    let x = v.get(key)?;
    x.as_f64().or_else(|| x.as_i64().map(|i| i as f64))
}

pub fn get_bool(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

/// `raw_json=1` already decodes entities, but be defensive with URLs that were
/// fetched without it.
fn unescape_amp(s: &str) -> String {
    s.replace("&amp;", "&")
}

fn ts_to_iso(ts: i64) -> Option<String> {
    if ts == 0 {
        return None;
    }
    DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.to_rfc3339())
}

/// Field of `d`, falling back to the crosspost parent (crossposts often carry
/// their media only on the original post).
fn field<'a>(d: &'a Value, parent: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    d.get(key).or_else(|| parent.and_then(|p| p.get(key)))
}

/// File extension for a reddit mime type, when it maps to a known one.
pub fn ext_from_mime(mime: &str) -> Option<String> {
    let ext = match mime {
        "image/jpg" | "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        _ => return None,
    };
    Some(ext.to_string())
}

/// Lowercase file extension of a URL path (query string ignored).
pub fn ext_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("");
    let (_, ext) = name.rsplit_once('.')?;
    let ext: String = ext
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    (2..=5).contains(&ext.len()).then_some(ext)
}

/// True when the URL path looks like a directly downloadable image.
pub fn is_image_url(url: &str) -> bool {
    ext_from_url(url).is_some_and(|ext| {
        matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"
        )
    })
}

/// [`MediaFormat`] of a URL path extension (`?query`/`#fragment` ignored).
pub fn format_from_url(url: &str) -> Option<MediaFormat> {
    MediaFormat::from_ext(&ext_from_url(url)?)
}

/// Format a URL's `format=` query parameter asks reddit's image resizer for
/// (`format=pjpg` → JPEG). This is what the server actually sends, which can
/// differ from the path extension (e.g. `….png?format=pjpg`).
pub fn format_from_query(url: &str) -> Option<MediaFormat> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key.eq_ignore_ascii_case("format") {
            return MediaFormat::from_ext(value);
        }
    }
    None
}

/// [`MediaFormat`] of a mime type (`image/jpeg; charset=…` accepted).
pub fn format_from_mime(mime: &str) -> Option<MediaFormat> {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    MediaFormat::from_ext(&ext_from_mime(base)?)
}

/// Derive the original `i.redd.it` URL of a gallery item from its media id.
fn original_gallery_url(media_id: &str, mime: Option<&str>, media_base: &str) -> Option<String> {
    let ext = ext_from_mime(mime?)?;
    Some(format!(
        "{}/{media_id}.{ext}",
        media_base.trim_end_matches('/')
    ))
}

/// Highest-resolution preview image of a listing (`preview.images[0].source`).
fn preview_image(preview: Option<&Value>) -> Option<MediaAsset> {
    let src = preview?.pointer("/images/0/source")?;
    let url = get_str(src, "url").map(|u| unescape_amp(&u))?;
    Some(MediaAsset {
        url,
        fallback: None,
        width: get_i64(src, "width"),
        height: get_i64(src, "height"),
        mime: None,
    })
}

/// Reddit-hosted video (`media.reddit_video` or `secure_media.reddit_video`).
fn reddit_video(media: Option<&Value>) -> Option<VideoInfo> {
    let rv = media?.get("reddit_video")?;
    let url = get_str(rv, "fallback_url").map(|u| unescape_amp(&u))?;
    Some(VideoInfo {
        url,
        width: get_i64(rv, "width"),
        height: get_i64(rv, "height"),
        duration: get_f64(rv, "duration"),
    })
}

/// Gallery images in display order. Each item keeps its original URL when it
/// can be derived and the signed preview URL as a fallback.
pub fn clean_gallery(d: &Value, parent: Option<&Value>, media_base: &str) -> Vec<GalleryItem> {
    let Some(items) = field(d, parent, "gallery_data")
        .and_then(|g| g.get("items"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let meta = field(d, parent, "media_metadata").and_then(|m| m.as_object());
    let mut out = Vec::with_capacity(items.len());

    for (index, item) in items.iter().enumerate() {
        let Some(media_id) = get_str(item, "media_id") else {
            continue;
        };
        let m = meta.and_then(|m| m.get(&media_id));
        let mime = m.and_then(|m| get_str(m, "m"));
        let e = m.and_then(|m| get_str(m, "e"));
        let s = m.and_then(|m| m.get("s"));
        let width = s.and_then(|s| get_i64(s, "x"));
        let height = s.and_then(|s| get_i64(s, "y"));
        let still = s.and_then(|s| get_str(s, "u")).map(|u| unescape_amp(&u));
        let mp4 = s.and_then(|s| get_str(s, "mp4")).map(|u| unescape_amp(&u));

        let is_video = mime.as_deref().is_some_and(|m| m.starts_with("video/"))
            || e.as_deref() == Some("Video");
        let animated = mp4.is_some() && !is_video;
        let kind = if is_video {
            "video"
        } else if animated {
            "gif"
        } else {
            "image"
        };

        let image_mime = mime
            .as_deref()
            .filter(|m| m.starts_with("image/"))
            .map(str::to_string);
        let image = match (
            original_gallery_url(&media_id, mime.as_deref(), media_base),
            still,
        ) {
            (Some(url), Some(fallback)) => Some(MediaAsset {
                url,
                fallback: Some(fallback),
                width,
                height,
                mime: image_mime,
            }),
            (Some(url), None) => Some(MediaAsset {
                url,
                fallback: None,
                width,
                height,
                mime: image_mime,
            }),
            (None, Some(url)) => Some(MediaAsset {
                url,
                fallback: None,
                width,
                height,
                mime: image_mime,
            }),
            (None, None) => None,
        };
        let video = mp4.map(|url| VideoInfo {
            url,
            width,
            height,
            duration: None,
        });
        if image.is_none() && video.is_none() {
            continue;
        }
        out.push(GalleryItem {
            index,
            media_id,
            kind: kind.to_string(),
            image,
            video,
            caption: get_str(item, "caption").filter(|c| !c.is_empty()),
        });
    }
    out
}

/// Map one raw listing item (`kind == "t3"` data object) to a [`Post`].
pub fn clean_post(d: &Value, media_base: &str) -> Post {
    let parent = d.pointer("/crosspost_parent_list/0");
    let get = |key: &str| field(d, parent, key);

    let created = get_f64(d, "created_utc").unwrap_or(0.0) as i64;
    let permalink = get_str(d, "permalink")
        .map(|p| format!("https://www.reddit.com{p}"))
        .unwrap_or_default();

    Post {
        id: get_str(d, "id").unwrap_or_default(),
        name: get_str(d, "name"),
        title: get_str(d, "title").unwrap_or_default(),
        author: get_str(d, "author").unwrap_or_default(),
        subreddit: get_str(d, "subreddit").unwrap_or_default(),
        permalink,
        url: get_str(d, "url").map(|u| unescape_amp(&u)),
        domain: get_str(d, "domain"),
        post_hint: get_str(d, "post_hint"),
        created_utc: created,
        datetime: ts_to_iso(created),
        selftext: get_str(d, "selftext").filter(|s| !s.is_empty()),
        score: get_i64(d, "score"),
        upvote_ratio: get_f64(d, "upvote_ratio"),
        num_comments: get_i64(d, "num_comments"),
        num_crossposts: get_i64(d, "num_crossposts"),
        over_18: get_bool(d, "over_18").unwrap_or(false),
        spoiler: get_bool(d, "spoiler").unwrap_or(false),
        stickied: get_bool(d, "stickied").unwrap_or(false),
        locked: get_bool(d, "locked").unwrap_or(false),
        is_self: get_bool(d, "is_self").unwrap_or(false),
        is_video: get_bool(d, "is_video").unwrap_or(false),
        is_gallery: get_bool(d, "is_gallery").unwrap_or(false),
        link_flair_text: get_str(d, "link_flair_text"),
        author_flair_text: get_str(d, "author_flair_text"),
        thumbnail: get_str(d, "thumbnail").filter(|t| t.starts_with("http")),
        preview: preview_image(get("preview")),
        video: reddit_video(get("media")).or_else(|| reddit_video(get("secure_media"))),
        gallery: clean_gallery(d, parent, media_base),
    }
}

fn first_url(candidates: [Option<String>; 2]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|u| u.starts_with("http"))
}

/// Map the `about.json` envelope (`{kind: "t5", data: {...}}`) to
/// [`SubredditInfo`].
pub fn clean_about(v: &Value) -> SubredditInfo {
    let d = v.get("data").unwrap_or(v);
    let created = get_f64(d, "created_utc").unwrap_or(0.0) as i64;
    SubredditInfo {
        name: get_str(d, "display_name").unwrap_or_default(),
        title: get_str(d, "title").filter(|s| !s.is_empty()),
        description: get_str(d, "public_description").filter(|s| !s.is_empty()),
        subscribers: get_i64(d, "subscribers"),
        created_utc: (created != 0).then_some(created),
        datetime: ts_to_iso(created),
        over_18: get_bool(d, "over18").unwrap_or(false),
        icon: first_url([get_str(d, "community_icon"), get_str(d, "icon_img")]),
        banner: first_url([
            get_str(d, "banner_background_image"),
            get_str(d, "banner_img"),
        ]),
        primary_color: get_str(d, "primary_color").filter(|s| !s.is_empty()),
    }
}

/// The image (and optional original URL fallback) that represents a
/// non-gallery post.
fn post_image(p: &Post) -> Option<MediaAsset> {
    let direct = p.url.as_deref().filter(|u| is_image_url(u));
    match (direct, &p.preview) {
        (Some(url), Some(preview)) => Some(MediaAsset {
            url: url.to_string(),
            fallback: Some(preview.url.clone()),
            width: preview.width,
            height: preview.height,
            mime: None,
        }),
        (Some(url), None) => Some(MediaAsset {
            url: url.to_string(),
            fallback: None,
            width: None,
            height: None,
            mime: None,
        }),
        (None, Some(preview)) => Some(preview.clone()),
        (None, None) => p
            .thumbnail
            .as_deref()
            .filter(|u| is_image_url(u))
            .map(|url| MediaAsset {
                url: url.to_string(),
                fallback: None,
                width: None,
                height: None,
                mime: None,
            }),
    }
}

fn asset_ext(asset: &MediaAsset) -> Option<String> {
    asset
        .mime
        .as_deref()
        .and_then(ext_from_mime)
        .or_else(|| format_from_query(&asset.url).map(|f| f.name().to_string()))
        .or_else(|| ext_from_url(&asset.url))
}

fn video_item(id: &str, index: Option<usize>, v: &VideoInfo) -> ManifestItem {
    ManifestItem {
        folder: "posts".into(),
        id: id.to_string(),
        kind: "video".into(),
        url: v.url.clone(),
        fallback: None,
        ext: Some("mp4".into()),
        index,
        width: v.width,
        height: v.height,
    }
}

fn art_item(kind: &str, url: &str) -> ManifestItem {
    ManifestItem {
        folder: String::new(),
        id: "subreddit".into(),
        kind: kind.into(),
        url: url.to_string(),
        fallback: None,
        ext: format_from_query(url)
            .map(|f| f.name().to_string())
            .or_else(|| ext_from_url(url)),
        index: None,
        width: None,
        height: None,
    }
}

/// Build the list of files to download. `gallery_cap` of `0` means every
/// gallery image; `include_videos` adds reddit-hosted/video files.
pub fn build_manifest(
    about: Option<&SubredditInfo>,
    posts: &[Post],
    include_videos: bool,
    gallery_cap: usize,
    skip_icon: bool,
) -> Vec<ManifestItem> {
    let mut out = Vec::new();

    if !skip_icon && let Some(a) = about {
        if let Some(url) = &a.icon {
            out.push(art_item("icon", url));
        }
        if let Some(url) = &a.banner {
            out.push(art_item("banner", url));
        }
    }

    for p in posts {
        if !p.gallery.is_empty() {
            for g in &p.gallery {
                if gallery_cap > 0 && g.index >= gallery_cap {
                    break;
                }
                if let Some(img) = &g.image {
                    out.push(ManifestItem {
                        folder: "posts".into(),
                        id: p.id.clone(),
                        kind: "gallery".into(),
                        url: img.url.clone(),
                        fallback: img.fallback.clone(),
                        ext: asset_ext(img),
                        index: Some(g.index),
                        width: img.width,
                        height: img.height,
                    });
                }
                if include_videos && let Some(v) = &g.video {
                    out.push(video_item(&p.id, Some(g.index), v));
                }
            }
        } else if let Some(img) = post_image(p) {
            out.push(ManifestItem {
                folder: "posts".into(),
                id: p.id.clone(),
                kind: "image".into(),
                url: img.url.clone(),
                fallback: img.fallback.clone(),
                ext: asset_ext(&img),
                index: None,
                width: img.width,
                height: img.height,
            });
        }
        if include_videos && let Some(v) = &p.video {
            out.push(video_item(&p.id, None, v));
        }
    }
    out
}

/// Format an item will be stored as. A `format=` query parameter wins (it is
/// what the server sends), then reddit's mime-derived `ext`, then the URL
/// extension.
pub fn item_format(item: &ManifestItem) -> Option<MediaFormat> {
    format_from_query(&item.url)
        .or_else(|| item.ext.as_deref().and_then(MediaFormat::from_ext))
        .or_else(|| format_from_url(&item.url))
}

/// Keep only manifest items whose stored format is in `wanted`; an empty
/// selection keeps everything. Items without a recognizable format are dropped
/// when a selection is given, so `--formats gif` can never download something
/// else.
pub fn filter_manifest(manifest: Vec<ManifestItem>, wanted: &[MediaFormat]) -> Vec<ManifestItem> {
    if wanted.is_empty() {
        return manifest;
    }
    manifest
        .into_iter()
        .filter(|item| item_format(item).is_some_and(|f| wanted.contains(&f)))
        .collect()
}

/// Replace characters that are awkward in file names and trim to `maxlen`.
pub fn sanitize_filename(s: &str, maxlen: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    let out = if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned
    };
    out.chars().take(maxlen).collect()
}

/// Parse `'k=v; k2=v2'` or a Netscape `cookies.txt` file. `#HttpOnly_` lines
/// (used by browsers for session cookies such as `reddit_session`) are read as
/// regular cookies instead of being skipped as comments.
pub fn parse_cookies(arg: &str) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    if Path::new(arg).is_file() {
        if let Ok(content) = std::fs::read_to_string(arg) {
            for line in content.lines() {
                let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 7 {
                    cookies.insert(parts[5].to_string(), parts[6].to_string());
                }
            }
        }
        return cookies;
    }
    for pair in arg.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            cookies.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    cookies
}

pub fn parse_since(s: &str) -> Result<i64> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("invalid --since date '{s}' (expected YYYY-MM-DD): {e}"))?;
    let midnight = date.and_hms_opt(0, 0, 0).expect("valid time");
    Ok(midnight.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gallery_value() -> Value {
        serde_json::json!({
            "id": "abc123",
            "name": "t3_abc123",
            "title": "A gallery",
            "author": "someone",
            "subreddit": "testsub",
            "permalink": "/r/testsub/comments/abc123/a_gallery/",
            "url": "https://www.reddit.com/gallery/abc123",
            "domain": "old.reddit.com",
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
    fn cookie_string_and_netscape_file() {
        let c = parse_cookies("a=1; b=two; x=y");
        assert_eq!(c.get("a").map(String::as_str), Some("1"));
        assert_eq!(c.get("b").map(String::as_str), Some("two"));
        assert_eq!(c.get("x").map(String::as_str), Some("y"));

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cookies.txt");
        std::fs::write(
            &p,
            "# Netscape HTTP Cookie File\n\
             #HttpOnly_.reddit.com\tTRUE\t/\tTRUE\t0\treddit_session\tSID\n\
             .reddit.com\tTRUE\t/\tFALSE\t0\tcsrf_token\tTOK\n",
        )
        .unwrap();
        let c = parse_cookies(p.to_str().unwrap());
        assert_eq!(c.get("reddit_session").map(String::as_str), Some("SID"));
        assert_eq!(c.get("csrf_token").map(String::as_str), Some("TOK"));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn url_extensions() {
        assert_eq!(
            ext_from_url("https://i.redd.it/x.jpeg?width=1&s=a"),
            Some("jpeg".into())
        );
        assert_eq!(
            ext_from_url("https://v.redd.it/x/CMAF_720.mp4"),
            Some("mp4".into())
        );
        assert_eq!(ext_from_url("https://example.com/noext"), None);
        assert!(is_image_url("https://preview.redd.it/a.webp?s=1"));
        assert!(!is_image_url("https://v.redd.it/a.mp4"));
    }

    #[test]
    fn gallery_cleaning_keeps_order_originals_and_fallbacks() {
        let post = clean_post(&gallery_value(), "https://i.redd.it");
        assert_eq!(post.id, "abc123");
        assert_eq!(post.created_utc, 1700100000);
        assert_eq!(post.gallery.len(), 3);
        assert_eq!(post.gallery[0].kind, "image");
        assert_eq!(
            post.gallery[0].image.as_ref().unwrap().url,
            "https://i.redd.it/m0.jpg"
        );
        assert_eq!(
            post.gallery[0].image.as_ref().unwrap().fallback.as_deref(),
            Some("https://preview.redd.it/m0.jpg?width=1000&format=pjpg")
        );
        assert_eq!(post.gallery[1].kind, "gif");
        assert_eq!(post.gallery[1].caption.as_deref(), Some("second"));
        assert_eq!(post.gallery[2].kind, "video");
        assert!(post.gallery[2].video.is_some());
    }

    #[test]
    fn gallery_uses_preview_when_no_original_is_derivable() {
        let mut v = gallery_value();
        v["media_metadata"]["m0"]["m"] = serde_json::json!("application/octet-stream");
        let post = clean_post(&v, "https://i.redd.it");
        let img = post.gallery[0].image.as_ref().unwrap();
        assert_eq!(
            img.url,
            "https://preview.redd.it/m0.jpg?width=1000&format=pjpg"
        );
        assert!(img.fallback.is_none());
    }

    #[test]
    fn crosspost_parent_media_is_used() {
        let v = serde_json::json!({
            "id": "cross1",
            "title": "crosspost",
            "is_gallery": true,
            "crosspost_parent_list": [gallery_value()],
        });
        let post = clean_post(&v, "https://i.redd.it");
        assert_eq!(post.gallery.len(), 3);
        assert!(post.preview.is_none());
    }

    #[test]
    fn post_images_and_videos() {
        let v = serde_json::json!({
            "id": "img1",
            "title": "direct image",
            "url": "https://i.redd.it/img1.jpeg",
            "thumbnail": "https://b.thumbs.redditmedia.com/t.jpg",
            "preview": {"images": [{"source": {"url": "https://preview.redd.it/img1.jpeg?s=1", "width": 1080, "height": 1080}}]},
            "is_video": true,
            "media": {"reddit_video": {"fallback_url": "https://v.redd.it/img1/DASH_720.mp4?source=fallback", "width": 720, "height": 1280, "duration": 12.0}}
        });
        let post = clean_post(&v, "https://i.redd.it");
        assert_eq!(post.media_kind(), "video");
        assert_eq!(
            post.preview.as_ref().unwrap().url,
            "https://preview.redd.it/img1.jpeg?s=1"
        );
        let video = post.video.as_ref().unwrap();
        assert_eq!(video.width, Some(720));
        assert_eq!(video.duration, Some(12.0));

        let m = build_manifest(None, std::slice::from_ref(&post), true, 0, false);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].kind, "image");
        assert_eq!(m[0].url, "https://i.redd.it/img1.jpeg");
        assert_eq!(
            m[0].fallback.as_deref(),
            Some("https://preview.redd.it/img1.jpeg?s=1")
        );
        assert_eq!(m[1].kind, "video");
    }

    #[test]
    fn manifest_order_gallery_cap_and_subreddit_art() {
        let post = clean_post(&gallery_value(), "https://i.redd.it");
        let about = SubredditInfo {
            name: "testsub".into(),
            icon: Some("https://styles.redditmedia.com/icon.png?w=256".into()),
            banner: Some("https://styles.redditmedia.com/banner.jpg?w=1".into()),
            ..SubredditInfo::default()
        };

        let m = build_manifest(Some(&about), std::slice::from_ref(&post), false, 0, false);
        let kinds: Vec<&str> = m.iter().map(|x| x.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["icon", "banner", "gallery", "gallery", "gallery"]
        );
        assert_eq!(m[0].folder, "");
        assert_eq!(m[2].folder, "posts");
        assert_eq!(m[2].ext.as_deref(), Some("jpg"));
        assert_eq!(m[3].ext.as_deref(), Some("png"));
        assert_eq!(m[4].ext.as_deref(), Some("mp4"));
        assert_eq!(m[4].index, Some(2));

        let capped = build_manifest(None, std::slice::from_ref(&post), false, 2, true);
        assert_eq!(capped.len(), 2);
        let skipped = build_manifest(Some(&about), std::slice::from_ref(&post), false, 0, true);
        assert!(
            !skipped
                .iter()
                .any(|x| x.kind == "icon" || x.kind == "banner")
        );
    }

    #[test]
    fn about_cleaning() {
        let v = serde_json::json!({
            "kind": "t5",
            "data": {
                "display_name": "testsub",
                "title": "Test",
                "public_description": "hello",
                "subscribers": 1000,
                "created_utc": 1600000000.0,
                "over18": true,
                "community_icon": "",
                "icon_img": "https://styles.redditmedia.com/icon.png?w=256",
                "banner_background_image": "https://styles.redditmedia.com/banner.jpg"
            }
        });
        let a = clean_about(&v);
        assert_eq!(a.name, "testsub");
        assert_eq!(a.subscribers, Some(1000));
        assert!(a.over_18);
        assert_eq!(
            a.icon.as_deref(),
            Some("https://styles.redditmedia.com/icon.png?w=256")
        );
        assert_eq!(a.datetime.as_deref(), Some("2020-09-13T12:26:40+00:00"));
    }

    #[test]
    fn since_dates() {
        assert_eq!(parse_since("2026-08-01").unwrap(), 1785542400);
        assert!(parse_since("01-08-2026").is_err());
        assert!(parse_since("garbage").is_err());
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(sanitize_filename("a/b:c*d", 80), "a_b_c_d");
        assert_eq!(sanitize_filename("  .  ", 80), "untitled");
        assert_eq!(sanitize_filename("verylongname", 5), "veryl");
    }

    #[test]
    fn media_formats_and_manifest_filtering() {
        assert_eq!(MediaFormat::from_ext("JPEG"), Some(MediaFormat::Jpg));
        assert_eq!(MediaFormat::from_ext("jpg"), MediaFormat::from_ext("jpeg"));
        assert_eq!(MediaFormat::from_ext("pjpg"), Some(MediaFormat::Jpg));
        assert_eq!(MediaFormat::from_ext("gifv"), None);
        assert_eq!(
            format_from_url("https://i.redd.it/a.GIF?s=1&format=pjpg"),
            Some(MediaFormat::Gif)
        );
        assert_eq!(format_from_url("https://example.com/noext"), None);
        assert_eq!(
            format_from_mime("image/jpeg; charset=utf-8"),
            Some(MediaFormat::Jpg)
        );
        assert_eq!(format_from_mime("application/octet-stream"), None);
        // reddit's resizer honours `format=` over the path extension
        assert_eq!(
            format_from_query("https://external-preview.redd.it/a.png?format=pjpg&auto=webp&s=1"),
            Some(MediaFormat::Jpg)
        );
        assert_eq!(format_from_query("https://i.redd.it/a.png"), None);
        assert_eq!(format_from_query("https://i.redd.it/a.png?width=100"), None);

        let preview = "https://external-preview.redd.it/x.png?format=pjpg&auto=webp&s=1";
        let item = |ext: Option<&str>, url: &str| ManifestItem {
            folder: "posts".into(),
            id: "x".into(),
            kind: "image".into(),
            url: url.into(),
            fallback: None,
            ext: ext.map(str::to_string),
            index: None,
            width: None,
            height: None,
        };
        let manifest = vec![
            item(Some("jpg"), "https://i.redd.it/a.jpg"),
            item(Some("gif"), "https://i.redd.it/a.gif"),
            item(None, "https://i.redd.it/b.png?s=1"),
            item(None, "https://example.com/opaque"),
            item(Some("png"), preview),
        ];

        assert_eq!(item_format(&manifest[2]), Some(MediaFormat::Png));
        assert_eq!(item_format(&manifest[3]), None);
        assert_eq!(item_format(&manifest[4]), Some(MediaFormat::Jpg));
        // no selection keeps everything, including unknown formats
        assert_eq!(filter_manifest(manifest.clone(), &[]).len(), 5);
        // gif only
        let gifs = filter_manifest(manifest.clone(), &[MediaFormat::Gif]);
        assert_eq!(gifs.len(), 1);
        assert_eq!(gifs[0].url, "https://i.redd.it/a.gif");
        // a jpeg selection matches `.jpg` and `?format=pjpg` previews
        let jpegs = filter_manifest(manifest.clone(), &[MediaFormat::Jpg]);
        assert_eq!(jpegs.len(), 2);
        assert_eq!(jpegs[1].url, preview);
        // a png selection must not claim the jpg preview
        assert_eq!(filter_manifest(manifest, &[MediaFormat::Png]).len(), 1);
    }

    #[test]
    fn external_preview_urls_follow_the_format_query() {
        let v = serde_json::json!({
            "id": "ext1",
            "title": "external link",
            "url": "https://external-preview.redd.it/abc.png?format=pjpg&auto=webp&s=1",
        });
        let post = clean_post(&v, "https://i.redd.it");
        let m = build_manifest(None, std::slice::from_ref(&post), false, 0, false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].ext.as_deref(), Some("jpg"));
        assert_eq!(item_format(&m[0]), Some(MediaFormat::Jpg));
    }
}
