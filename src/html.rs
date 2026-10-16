//! Self-contained offline viewer pages.
//!
//! Each archive gets an `index.html` (inline CSS/JS, no external resources)
//! with a post grid and a lightbox. Media loads from `media/` when
//! downloaded, then falls back to the remote URLs. The output root gets a hub
//! page linking every archive.

use crate::archive::ArchiveEntry;
use crate::models::{AssetKind, ManifestItem, Post, Sort, SubredditInfo, TimeFilter};
use crate::target::Target;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const ARCHIVE_TEMPLATE: &str = include_str!("viewer/archive.html");
const HUB_TEMPLATE: &str = include_str!("viewer/hub.html");
const DATA_PLACEHOLDER: &str = "__DATA__";

/// Run metadata shown in the viewer header.
#[derive(Clone, Copy, Debug)]
pub struct ViewerMeta<'a> {
    pub sort: Sort,
    pub time: TimeFilter,
    pub fetched_at: &'a str,
}

/// `(post id, kind, gallery index)`.
type MediaKey<'a> = (&'a str, AssetKind, Option<usize>);

/// Embed `data` as JSON; `</` is escaped so post text cannot close the
/// `<script>` element.
fn render<T: Serialize>(template: &str, data: &T) -> String {
    let json = serde_json::to_string(data)
        .expect("viewer data is always serializable")
        .replace("</", "<\\/");
    template.replacen(DATA_PLACEHOLDER, &json, 1)
}

fn media_ref(item: Option<&ManifestItem>) -> Value {
    item.map_or(Value::Null, |item| {
        json!({
            "local": item.web_path(),
            "remote": item.url,
            "fallback": item.fallback,
            "w": item.width,
            "h": item.height,
        })
    })
}

fn post_json(post: &Post, lookup: &HashMap<MediaKey<'_>, &ManifestItem>) -> Value {
    let get = |kind, index| lookup.get(&(post.id.as_str(), kind, index)).copied();
    let slides: Vec<Value> = if post.gallery.is_empty() {
        let (image, video) = (get(AssetKind::Image, None), get(AssetKind::Video, None));
        (image.is_some() || video.is_some())
            .then(|| json!({"img": media_ref(image), "video": media_ref(video)}))
            .into_iter()
            .collect()
    } else {
        post.gallery
            .iter()
            .filter_map(|item| {
                let image = get(AssetKind::Gallery, Some(item.index));
                let video = get(AssetKind::Video, Some(item.index));
                (image.is_some() || video.is_some())
                    .then(|| json!({"img": media_ref(image), "video": media_ref(video)}))
            })
            .collect()
    };
    let thumb = slides
        .iter()
        .map(|slide| &slide["img"])
        .chain(slides.first().map(|slide| &slide["video"]))
        .find(|media| !media.is_null())
        .cloned();
    let link = post
        .url
        .as_deref()
        .filter(|url| !url.is_empty() && *url != post.permalink);

    json!({
        "title": post.title,
        "author": post.author,
        "permalink": post.permalink,
        "link": link,
        "domain": post.domain,
        "date": post.datetime,
        "score": post.score,
        "ratio": post.upvote_ratio,
        "comments": post.num_comments,
        "flair": post.link_flair_text,
        "nsfw": post.over_18,
        "spoiler": post.spoiler,
        "stickied": post.stickied,
        "selftext": post.selftext,
        "type": post.media_kind(),
        "media": slides,
        "thumb": thumb,
    })
}

/// Render an archive's viewer. Items in `manifest` that were not downloaded
/// fall back to their remote URLs.
pub fn render_archive(
    target: &Target,
    about: Option<&SubredditInfo>,
    posts: &[Post],
    manifest: &[ManifestItem],
    meta: ViewerMeta<'_>,
) -> String {
    let lookup: HashMap<MediaKey<'_>, &ManifestItem> = manifest
        .iter()
        .map(|item| ((item.post_id.as_str(), item.kind, item.index), item))
        .collect();
    let art = |kind| media_ref(lookup.get(&("", kind, None)).copied());
    let sort = if meta.sort.uses_time() {
        format!("{} · {}", meta.sort.as_str(), meta.time.as_str())
    } else {
        meta.sort.as_str().to_owned()
    };

    render(
        ARCHIVE_TEMPLATE,
        &json!({
            "target": {
                "name": target.name(),
                "kind": target.kind().as_str(),
                "display": target.to_string(),
                "url": target.url(),
                "sort": sort,
                "fetched_at": meta.fetched_at,
                "hub": true,
            },
            "about": about.map(|a| json!({
                "title": a.title,
                "description": a.description,
                "subscribers": a.subscribers,
                "over18": a.over_18,
                "icon": art(AssetKind::Icon),
                "banner": art(AssetKind::Banner),
            })),
            "posts": posts.iter().map(|p| post_json(p, &lookup)).collect::<Vec<_>>(),
        }),
    )
}

/// Render the output-root hub, most recently fetched archives first.
pub fn render_hub(entries: &[ArchiveEntry]) -> String {
    let mut sorted: Vec<&ArchiveEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        b.fetched_at
            .cmp(&a.fetched_at)
            .then_with(|| a.dir.cmp(&b.dir))
    });
    let archives: Vec<Value> = sorted
        .into_iter()
        .map(|e| {
            json!({
                "dir": e.dir,
                "display": e.display,
                "title": e.title,
                "icon": e.icon,
                "iconRemote": e.icon_remote,
                "posts": e.posts,
                "fetchedAt": e.fetched_at,
                "over18": e.over_18,
                "viewer": e.viewer,
            })
        })
        .collect();
    render(HUB_TEMPLATE, &json!({ "archives": archives }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{self, Selection};
    use crate::normalize;

    fn gallery_post() -> Post {
        normalize::post(
            &json!({
                "id": "g1",
                "title": "gallery </script> post",
                "author": "someone",
                "permalink": "/r/testsub/comments/g1/gallery/",
                "url": "https://www.reddit.com/gallery/g1",
                "created_utc": 1700100000.0,
                "is_gallery": true,
                "gallery_data": {"items": [{"media_id": "a"}, {"media_id": "b"}]},
                "media_metadata": {
                    "a": {"m": "image/jpg", "s": {"u": "https://preview.redd.it/a.jpg", "x": 100, "y": 200}},
                    "b": {"m": "image/png", "s": {"u": "https://preview.redd.it/b.png", "x": 300, "y": 400}}
                }
            }),
            "https://i.redd.it",
        )
    }

    fn image_post() -> Post {
        normalize::post(
            &json!({
                "id": "i1",
                "title": "image post",
                "permalink": "/r/testsub/comments/i1/image/",
                "url": "https://i.redd.it/i1.jpeg",
                "preview": {"images": [{"source": {"url": "https://preview.redd.it/i1.jpeg?s=1", "width": 1080, "height": 1080}}]}
            }),
            "https://i.redd.it",
        )
    }

    fn meta() -> ViewerMeta<'static> {
        ViewerMeta {
            sort: Sort::Top,
            time: TimeFilter::Week,
            fetched_at: "2026-09-28T12:00:00+00:00",
        }
    }

    #[test]
    fn archive_page_embeds_posts_media_and_metadata() {
        let posts = [gallery_post(), image_post()];
        let manifest = manifest::build(None, &posts, &Selection::default(), None).items;
        let about = normalize::subreddit(&json!({
            "data": {"display_name": "testsub", "title": "Test Sub", "subscribers": 10, "over18": true}
        }));
        let target = Target::subreddit("testsub").unwrap();
        let html = render_archive(&target, Some(&about), &posts, &manifest, meta());

        for expected in [
            "r/testsub",
            "Test Sub",
            "top · week",
            "media/posts/g1_00.jpg",
            "media/posts/g1_01.png",
            "media/posts/i1.jpg",
            "https://preview.redd.it/a.jpg",
            "https://i.redd.it/i1.jpeg",
            "\"type\":\"gallery\"",
            "\"hub\":true",
            "<\\/script> post",
        ] {
            assert!(html.contains(expected), "missing {expected}");
        }
        assert!(
            !html.contains("</script> post"),
            "data must not close the script"
        );
        assert!(!html.contains(DATA_PLACEHOLDER));
    }

    #[test]
    fn hub_lists_archives_newest_first() {
        let entry = |dir: &str, fetched: &str, viewer| ArchiveEntry {
            dir: dir.into(),
            display: dir.replace('_', "/"),
            title: None,
            icon: Some(format!("{dir}/media/subreddit_icon.png")),
            icon_remote: None,
            posts: 3,
            fetched_at: Some(fetched.into()),
            over_18: false,
            viewer,
        };
        let html = render_hub(&[
            entry("r_old", "2026-01-01T00:00:00+00:00", false),
            entry("u_new", "2026-09-01T00:00:00+00:00", true),
        ]);
        let (new, old) = (
            html.find("\"dir\":\"u_new\""),
            html.find("\"dir\":\"r_old\""),
        );
        assert!(new.unwrap() < old.unwrap());
        assert!(html.contains("\"viewer\":false"));
        assert!(!html.contains(DATA_PLACEHOLDER));
        assert!(render_hub(&[]).contains("No archives yet"));
    }
}
