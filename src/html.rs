//! Offline HTML viewer.
//!
//! Renders the downloaded subreddit archive into a single self-contained
//! `index.html` (inline CSS and JavaScript, no external resources) that any
//! browser can open from disk — no network and no server required.
//!
//! The page shows the subreddit header, a grid of every saved post and a
//! lightbox for viewing images, galleries, self posts and (when downloaded)
//! videos. Media is served from the local `media/` directory when the file was
//! downloaded, otherwise it falls back to the original reddit CDN URL.

use crate::download::manifest_rel_path;
use crate::models::*;
use serde_json::{Value, json};
use std::collections::HashMap;

/// `(post id, manifest kind, gallery index)`.
type MediaKey = (String, String, Option<usize>);

/// Run metadata embedded in the viewer header.
pub struct ViewerMeta<'a> {
    pub sort: Sort,
    pub time: TimeFilter,
    pub fetched_at: &'a str,
}

fn build_lookup(manifest: &[ManifestItem]) -> HashMap<MediaKey, &ManifestItem> {
    let mut map = HashMap::new();
    for item in manifest {
        map.insert((item.id.clone(), item.kind.clone(), item.index), item);
    }
    map
}

fn media_ref(item: Option<&ManifestItem>) -> Value {
    match item {
        Some(item) => json!({
            "remote": item.url,
            "fallback": item.fallback,
            "local": manifest_rel_path(item).to_string_lossy().replace('\\', "/"),
            "w": item.width,
            "h": item.height,
        }),
        None => Value::Null,
    }
}

fn post_json(p: &Post, lookup: &HashMap<MediaKey, &ManifestItem>) -> Value {
    let get = |kind: &str, index: Option<usize>| {
        lookup
            .get(&(p.id.clone(), kind.to_string(), index))
            .copied()
    };

    let mut media: Vec<Value> = Vec::new();
    if !p.gallery.is_empty() {
        for g in &p.gallery {
            let img = media_ref(get("gallery", Some(g.index)));
            let video = media_ref(get("video", Some(g.index)));
            if img.is_null() && video.is_null() {
                continue;
            }
            media.push(json!({ "img": img, "video": video }));
        }
    } else {
        let img = get("image", None).or_else(|| get("thumb", None));
        let video = get("video", None);
        if img.is_some() || video.is_some() {
            media.push(json!({
                "img": media_ref(img),
                "video": media_ref(video),
            }));
        }
    }

    let thumb = media
        .iter()
        .find_map(|m| m.get("img").filter(|v| !v.is_null()).cloned())
        .or_else(|| {
            media
                .first()
                .and_then(|m| m.get("video"))
                .filter(|v| !v.is_null())
                .cloned()
        });

    let link = p
        .url
        .as_deref()
        .filter(|u| !u.is_empty() && Some(*u) != Some(p.permalink.as_str()));

    json!({
        "title": p.title,
        "author": p.author,
        "permalink": p.permalink,
        "link": link,
        "domain": p.domain,
        "date": p.datetime,
        "score": p.score,
        "ratio": p.upvote_ratio,
        "comments": p.num_comments,
        "flair": p.link_flair_text,
        "nsfw": p.over_18,
        "spoiler": p.spoiler,
        "stickied": p.stickied,
        "selftext": p.selftext,
        "type": p.media_kind(),
        "media": media,
        "thumb": thumb,
    })
}

/// Render the complete viewer page. `manifest` maps posts to their on-disk
/// files under `media/`; items that were not downloaded fall back to their
/// remote URLs.
pub fn render_index(
    target: &Target,
    about: Option<&SubredditInfo>,
    posts: &[Post],
    manifest: &[ManifestItem],
    meta: &ViewerMeta,
) -> String {
    let lookup = build_lookup(manifest);

    let sort_label = if meta.sort.uses_time() {
        format!("{} · {}", meta.sort.name(), meta.time.name())
    } else {
        meta.sort.name().to_string()
    };

    let icon = lookup.get(&("subreddit".to_string(), "icon".to_string(), None));
    let banner = lookup.get(&("subreddit".to_string(), "banner".to_string(), None));

    let about_json = about.map(|a| {
        json!({
            "title": a.title,
            "description": a.description,
            "subscribers": a.subscribers,
            "over18": a.over_18,
            "icon": media_ref(icon.copied()),
            "banner": media_ref(banner.copied()),
        })
    });

    let data = json!({
        "target": {
            "name": target.name,
            "kind": target.kind.name(),
            "display": target.display(),
            "url": target.url(),
            "sort": sort_label,
            "fetched_at": meta.fetched_at,
        },
        "about": about_json,
        "posts": posts.iter().map(|p| post_json(p, &lookup)).collect::<Vec<_>>(),
    });

    let json_str = serde_json::to_string(&data)
        .expect("viewer data serializes")
        .replace("</", "<\\/");
    PAGE_TEMPLATE.replace("__DATA__", &json_str)
}

const PAGE_TEMPLATE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>reddit — offline archive</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin: 0; font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: #0f0f0f; color: #f5f5f5; }
  header.sub { position: relative; }
  .banner { height: 150px; background-size: cover; background-position: center; border-bottom: 1px solid #262626; }
  .headrow { display: flex; gap: 18px; align-items: flex-start; padding: 22px 28px 18px; flex-wrap: wrap; }
  .icon { width: 76px; height: 76px; border-radius: 50%; object-fit: cover; background: #1a1a1a; flex-shrink: 0; }
  .icon.fallback { display: flex; align-items: center; justify-content: center; font-size: 30px; font-weight: 600; color: #fff; background: linear-gradient(135deg, #ff4500, #ff8717); user-select: none; }
  h1 { font-size: 21px; margin: 0; }
  h1 small { color: #8a8a8a; font-size: 14px; font-weight: 500; }
  .badge { background: #b91c1c; color: #fff; font-size: 11px; font-weight: 700; border-radius: 4px; padding: 2px 6px; vertical-align: 2px; }
  .stats { display: flex; gap: 20px; margin-top: 8px; font-size: 14px; color: #b5b5b5; flex-wrap: wrap; }
  .stats b { color: #fff; }
  .desc { margin: 8px 0 0; color: #a8a8a8; max-width: 680px; white-space: pre-line; font-size: 13px; line-height: 1.5; }
  main { padding: 18px 20px 40px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 8px; }
  .card { position: relative; aspect-ratio: 1; overflow: hidden; cursor: pointer; border-radius: 8px; background: #1a1a1a; }
  .card img { width: 100%; height: 100%; object-fit: cover; display: block; transition: transform .15s ease; }
  .card:hover img { transform: scale(1.03); }
  .card .tag { position: absolute; top: 8px; right: 8px; font-size: 12px; line-height: 1; background: rgba(0,0,0,.7); border-radius: 4px; padding: 5px 7px; }
  .cardlabel { position: absolute; left: 0; right: 0; bottom: 0; padding: 22px 10px 9px; font-size: 12.5px; line-height: 1.35; color: #fff; background: linear-gradient(transparent, rgba(0,0,0,.88)); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .card.text { display: flex; align-items: flex-end; background: #16202a; }
  .card.text .cardlabel { -webkit-line-clamp: 5; }
  .empty { color: #8a8a8a; text-align: center; padding: 56px 0; }
  footer { padding: 16px 28px 28px; color: #565656; font-size: 12px; }
  .modal { position: fixed; inset: 0; background: rgba(0,0,0,.88); display: none; align-items: center; justify-content: center; z-index: 10; padding: 24px; }
  .modal.open { display: flex; }
  .box { background: #181818; border-radius: 12px; width: min(940px, 96vw); height: min(92vh, 92dvh); display: flex; flex-direction: column; overflow: hidden; }
  .stage { position: relative; flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; background: #000; }
  .media-wrap { position: relative; display: flex; align-items: center; justify-content: center; min-width: 320px; min-height: 240px; max-width: 100%; max-height: 100%; }
  .media-wrap img, .media-wrap video { max-width: 100%; max-height: 100%; object-fit: contain; }
  .skeleton { position: absolute; inset: 0; background: linear-gradient(90deg, #161616 25%, #262626 37%, #161616 63%); background-size: 400% 100%; animation: shimmer 1.3s ease infinite; transition: opacity .3s ease; }
  .skeleton.done { opacity: 0; }
  @keyframes shimmer { 0% { background-position: 100% 0; } 100% { background-position: 0 0; } }
  .arrow { position: absolute; top: 50%; transform: translateY(-50%); background: rgba(255,255,255,.12); color: #fff; border: none; border-radius: 50%; width: 38px; height: 38px; font-size: 20px; cursor: pointer; }
  .arrow.prev { left: 10px; } .arrow.next { right: 10px; }
  .arrow:hover { background: rgba(255,255,255,.25); }
  .info { flex-shrink: 1; min-height: 0; max-height: 45%; padding: 14px 18px 16px; overflow-y: auto; }
  .ptitle { margin: 0 0 8px; font-size: 17px; line-height: 1.35; }
  .ptitle a { color: #fff; text-decoration: none; }
  .ptitle a:hover { text-decoration: underline; }
  .selftext { margin: 0 0 10px; white-space: pre-line; font-size: 14px; line-height: 1.5; color: #d6d6d6; }
  .meta { color: #8a8a8a; font-size: 13px; display: flex; gap: 14px; flex-wrap: wrap; align-items: center; }
  .meta a { color: #ff8717; text-decoration: none; }
  .close { position: absolute; top: 10px; right: 12px; background: rgba(0,0,0,.55); border: none; color: #fff; width: 36px; height: 36px; border-radius: 50%; font-size: 22px; line-height: 1; cursor: pointer; z-index: 3; }
  .close:hover { background: rgba(255,255,255,.25); }
  button { -webkit-tap-highlight-color: transparent; touch-action: manipulation; }
  @media (max-width: 640px) {
    .modal { padding: 0; background: #000; }
    .box { width: 100vw; height: 100dvh; border-radius: 0; }
    .arrow { width: 44px; height: 44px; font-size: 22px; }
    .close { width: 44px; height: 44px; top: 12px; right: 14px; font-size: 26px; }
    .headrow { padding: 16px; gap: 14px; }
    .icon { width: 58px; height: 58px; font-size: 24px; }
    .banner { height: 96px; }
    h1 { font-size: 18px; }
    main { padding: 12px 10px 28px; }
    .grid { grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 4px; }
    .cardlabel { font-size: 11px; padding: 16px 8px 7px; }
  }
</style>
</head>
<body>
<header class="sub" id="head"></header>
<main id="view"></main>
<footer>Generated by the <b>reddit</b> offline viewer · github.com/reddit-rs/reddit</footer>
<div class="modal" id="modal" tabindex="-1">
  <div class="box">
    <div class="stage" id="stage"><button class="close" id="close" title="Close (Esc)">&times;</button></div>
    <div class="info" id="info"></div>
  </div>
</div>
<script type="application/json" id="rd-data">__DATA__</script><script>
"use strict";
const DATA = JSON.parse(document.getElementById("rd-data").textContent);
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const src = (m) => m ? (m.local || m.remote) : null;
const mediaChain = (m) => m ? [m.local, m.remote, m.fallback].filter(Boolean) : [];
// load a media element, walking local file -> remote URL -> fallback URL
function loadMedia(el, m, onReady) {
  const chain = mediaChain(m);
  let idx = 0;
  let done = false;
  const finish = () => { if (!done) { done = true; onReady(); } };
  el.addEventListener("load", finish);
  el.addEventListener("canplay", finish);
  el.addEventListener("error", () => {
    idx += 1;
    if (idx < chain.length) { el.src = chain[idx]; } else { finish(); }
  });
  if (chain.length) { el.src = chain[0]; } else { finish(); }
}
const fmt = (n) => n == null ? "" : (n >= 1000 ? (n / 1000).toFixed(1).replace(/\.0$/, "") + "K" : String(n));
const fmtDate = (iso) => iso ? new Date(iso).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "";
const state = { post: 0, slide: 0 };

document.title = DATA.target.display + " — offline archive";

(function renderHead() {
  const a = DATA.about || {};
  const icon = src(a.icon);
  const banner = src(a.banner);
  const stats = [
    a.subscribers != null ? "<span><b>" + fmt(a.subscribers) + "</b> members</span>" : "",
    "<span><b>" + DATA.posts.length + "</b> posts saved</span>",
    DATA.target.sort ? "<span>sorted by <b>" + esc(DATA.target.sort) + "</b></span>" : "",
    DATA.target.fetched_at ? "<span>fetched " + esc(fmtDate(DATA.target.fetched_at)) + "</span>" : "",
  ].filter(Boolean).join("");
  $("head").innerHTML =
    (banner ? '<div class="banner" style="background-image:url(' + esc(banner) + ')"></div>' : "") +
    '<div class="headrow">' +
      (icon
        ? '<img class="icon" src="' + esc(icon) + '" alt="">'
        : '<div class="icon fallback">' + esc((a.title || DATA.target.name || "?").charAt(0).toUpperCase()) + '</div>') +
      '<div><h1>' + esc(DATA.target.display) +
        (a.title && a.title !== DATA.target.name ? ' <small>' + esc(a.title) + '</small>' : "") +
        (a.over18 ? ' <span class="badge">18+</span>' : "") +
      '</h1>' +
      '<div class="stats">' + stats + '</div>' +
      (a.description ? '<p class="desc">' + esc(a.description) + '</p>' : "") +
      '</div>' +
    '</div>';
})();

function slidesOf(it) {
  return (it.media || []).filter((m) => src(m.video) || src(m.img));
}

function card(it, i) {
  const thumb = src(it.thumb);
  const tag = it.type === "gallery" && it.media.length > 1 ? "&#9636; " + it.media.length
    : it.type === "video" ? "&#9654;"
    : it.nsfw ? "18+"
    : "";
  const label = '<div class="cardlabel">' + esc(it.title) + '</div>';
  const tagHtml = tag ? '<span class="tag">' + tag + '</span>' : "";
  return thumb
    ? '<div class="card" data-i="' + i + '"><img loading="lazy" src="' + esc(thumb) + '" alt="">' + tagHtml + label + '</div>'
    : '<div class="card text" data-i="' + i + '">' + tagHtml + label + '</div>';
}

function renderView() {
  $("view").innerHTML = DATA.posts.length
    ? '<div class="grid">' + DATA.posts.map(card).join("") + '</div>'
    : '<div class="empty">No posts saved.</div>';
  document.querySelectorAll("#view .card img").forEach((img) => {
    const it = DATA.posts[Number(img.closest(".card").dataset.i)];
    if (!it || !it.thumb) return;
    const chain = mediaChain(it.thumb);
    let idx = Math.max(0, chain.indexOf(img.getAttribute("src")));
    img.addEventListener("error", () => {
      idx += 1;
      if (idx < chain.length) img.src = chain[idx];
    });
  });
}

function openModal(i, slide) {
  const it = DATA.posts[i];
  if (!it) return;
  state.post = i;
  if (slide != null) state.slide = slide;
  const slides = slidesOf(it);
  if (!slides.length) state.slide = 0;
  const s = slides.length ? slides[state.slide % slides.length] : null;
  const stage = $("stage");
  stage.innerHTML = '<button class="close" id="close" title="Close (Esc)">&times;</button>';

  if (s) {
    const wrap = document.createElement("div");
    wrap.className = "media-wrap";
    const w = (s.video && s.video.w) || (s.img && s.img.w);
    const h = (s.video && s.video.h) || (s.img && s.img.h);
    if (w && h) {
      wrap.style.aspectRatio = (w / h).toFixed(4);
      wrap.style.minWidth = "0";
      wrap.style.minHeight = "0";
    }
    wrap.innerHTML = '<div class="skeleton"></div>';
    const ready = () => {
      const media = wrap.querySelector("img, video");
      if (media) media.style.opacity = "1";
      const sk = wrap.querySelector(".skeleton");
      if (sk) { sk.classList.add("done"); setTimeout(() => sk.remove(), 400); }
    };
    const useVideo = s.video && (s.video.local || !s.img);
    if (useVideo) {
      const v = document.createElement("video");
      v.controls = true;
      v.autoplay = true;
      v.style.opacity = "0";
      v.style.transition = "opacity .3s ease";
      wrap.appendChild(v);
      loadMedia(v, s.video, ready);
    } else if (s.img) {
      const img = document.createElement("img");
      img.alt = "";
      img.style.opacity = "0";
      img.style.transition = "opacity .3s ease";
      wrap.appendChild(img);
      loadMedia(img, s.img, ready);
    }
    stage.appendChild(wrap);
    if (slides.length > 1) {
      stage.insertAdjacentHTML(
        "beforeend",
        '<button class="arrow prev" title="Previous (&#8592;)">&#8249;</button><button class="arrow next" title="Next (&#8594;)">&#8250;</button>'
      );
    }
  } else {
    stage.insertAdjacentHTML("beforeend", '<div class="empty">No media saved for this post.</div>');
  }

  const meta = [
    it.author ? "<span>u/" + esc(it.author) + "</span>" : "",
    it.score != null ? "<span>&#9650; " + fmt(it.score) + "</span>" : "",
    it.comments != null ? "<span>&#128172; " + fmt(it.comments) + "</span>" : "",
    it.date ? "<span>&#128337; " + fmtDate(it.date) + "</span>" : "",
    it.flair ? "<span>" + esc(it.flair) + "</span>" : "",
    it.nsfw ? "<span>18+</span>" : "",
    it.spoiler ? "<span>spoiler</span>" : "",
  ].filter(Boolean).join("");
  $("info").innerHTML =
    '<h2 class="ptitle"><a href="' + esc(it.permalink) + '" target="_blank" rel="noopener">' + esc(it.title) + '</a></h2>' +
    (it.selftext ? '<p class="selftext">' + esc(it.selftext) + '</p>' : "") +
    '<div class="meta">' + meta +
      (it.link ? '<a href="' + esc(it.link) + '" target="_blank" rel="noopener">linked site &#8599;</a>' : "") +
    '</div>';

  stage.querySelector(".close").onclick = closeModal;
  const prev = stage.querySelector(".prev");
  const next = stage.querySelector(".next");
  if (prev) prev.onclick = (e) => { e.stopPropagation(); state.slide = (state.slide - 1 + slides.length) % slides.length; openModal(state.post, state.slide); };
  if (next) next.onclick = (e) => { e.stopPropagation(); state.slide = (state.slide + 1) % slides.length; openModal(state.post, state.slide); };
  const v = stage.querySelector("video");
  if (v) v.addEventListener("keydown", (e) => { if (e.key === "Escape") closeModal(); });

  $("modal").classList.add("open");
  $("modal").focus();
  $("modal").onclick = (e) => { if (e.target === $("modal")) closeModal(); };
}

function closeModal() {
  const v = $("stage").querySelector("video");
  if (v) { v.pause(); v.removeAttribute("src"); v.load(); }
  $("stage").innerHTML = "";
  $("info").innerHTML = "";
  $("modal").classList.remove("open");
  state.slide = 0;
}

$("view").addEventListener("click", (e) => {
  const c = e.target.closest(".card");
  if (!c) return;
  openModal(Number(c.dataset.i), 0);
});

document.addEventListener("keydown", (e) => {
  if (!$("modal").classList.contains("open")) return;
  if (e.key === "Escape") { closeModal(); return; }
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
  const it = DATA.posts[state.post];
  const slides = it ? slidesOf(it) : [];
  if (slides.length > 1) {
    state.slide = (state.slide + (e.key === "ArrowRight" ? 1 : slides.length - 1)) % slides.length;
    openModal(state.post, state.slide);
  }
});

renderView();
</script>
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::{clean_about, clean_post};

    fn gallery_post() -> Post {
        clean_post(
            &serde_json::json!({
                "id": "g1",
                "title": "gallery </script> post",
                "author": "someone",
                "permalink": "/r/testsub/comments/g1/gallery/",
                "url": "https://www.reddit.com/gallery/g1",
                "created_utc": 1700100000.0,
                "score": 5,
                "num_comments": 1,
                "over_18": true,
                "is_gallery": true,
                "selftext": "hello",
                "gallery_data": {"items": [{"media_id": "a"}, {"media_id": "b"}]},
                "media_metadata": {
                    "a": {"m": "image/jpg", "e": "Image", "s": {"u": "https://preview.redd.it/a.jpg", "x": 100, "y": 200}},
                    "b": {"m": "image/png", "e": "Image", "s": {"u": "https://preview.redd.it/b.png", "x": 300, "y": 400}}
                }
            }),
            "https://i.redd.it",
        )
    }

    fn image_post() -> Post {
        clean_post(
            &serde_json::json!({
                "id": "i1",
                "title": "image post",
                "author": "someone",
                "permalink": "/r/testsub/comments/i1/image/",
                "url": "https://i.redd.it/i1.jpeg",
                "created_utc": 1700000000.0,
                "thumbnail": "https://b.thumbs.redditmedia.com/t.jpg",
                "preview": {"images": [{"source": {"url": "https://preview.redd.it/i1.jpeg?s=1", "width": 1080, "height": 1080}}]}
            }),
            "https://i.redd.it",
        )
    }

    fn manifest(posts: &[Post]) -> Vec<ManifestItem> {
        crate::clean::build_manifest(None, posts, false, 0, false)
    }

    fn meta() -> ViewerMeta<'static> {
        ViewerMeta {
            sort: Sort::New,
            time: TimeFilter::All,
            fetched_at: "2026-09-28T12:00:00+00:00",
        }
    }

    #[test]
    fn page_contains_posts_media_and_local_paths() {
        let posts = vec![gallery_post(), image_post()];
        let m = manifest(&posts);
        let about = clean_about(&serde_json::json!({
            "data": {"display_name": "testsub", "title": "Test Sub", "public_description": "hi", "subscribers": 10, "over18": true}
        }));
        let html = render_index(
            &Target::subreddit("testsub"),
            Some(&about),
            &posts,
            &m,
            &meta(),
        );

        assert!(html.contains("r/testsub"));
        assert!(html.contains("Test Sub"));
        assert!(html.contains("media/posts/g1_00.jpg"));
        assert!(html.contains("media/posts/g1_01.png"));
        assert!(html.contains("media/posts/i1.jpeg"));
        assert!(html.contains("https://preview.redd.it/a.jpg")); // remote fallback
        assert!(html.contains("2026-09-28T12:00:00+00:00"));
        assert!(html.contains("\"type\":\"gallery\""));
        // `</script>` inside data must not break the page
        assert!(!html.contains("</script> post"));
        assert!(html.contains("<\\/script> post"));
        assert!(!html.contains("__DATA__"));
    }

    #[test]
    fn no_downloads_still_renders_remote_urls() {
        let posts = vec![image_post()];
        // manifest exists even without downloaded files (viewer falls back to remote)
        let m = manifest(&posts);
        let html = render_index(&Target::subreddit("testsub"), None, &posts, &m, &meta());
        assert!(html.contains("media/posts/i1.jpeg"));
        assert!(html.contains("https://i.redd.it/i1.jpeg"));
    }

    #[test]
    fn post_kind_label() {
        assert_eq!(image_post().media_kind(), "image");
        assert_eq!(gallery_post().media_kind(), "gallery");
    }
}
