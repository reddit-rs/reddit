//! End-to-end tests: the whole pipeline runs against a mocked reddit API and
//! mocked media hosts (no network access required).

use reddit::{Config, MediaFormat, PostsMode, Sort, Target, clean, models::ManifestItem};
use serde_json::{Value, json};
use tempfile::tempdir;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = reddit::UA_DEFAULT;

fn image_asset(server_uri: &str, code: &str) -> Value {
    json!({
        "status": "valid",
        "e": "Image",
        "m": "image/jpg",
        "s": {
            "u": format!("{server_uri}/prev/{code}.jpg?width=800&format=pjpg&s=sig"),
            "x": 800,
            "y": 1200
        }
    })
}

fn gallery_post(server_uri: &str) -> Value {
    json!({
        "id": "g1",
        "name": "t3_g1",
        "title": "Jane gallery",
        "author": "poster",
        "subreddit": "testsub",
        "permalink": "/r/testsub/comments/g1/jane_gallery/",
        "url": "https://www.reddit.com/gallery/g1",
        "domain": "old.reddit.com",
        "created_utc": 1700100000.0,
        "score": 100,
        "upvote_ratio": 0.9,
        "num_comments": 4,
        "over_18": true,
        "is_gallery": true,
        "link_flair_text": "Body",
        "gallery_data": {"items": [
            {"media_id": "m0", "id": 1},
            {"media_id": "m1", "caption": "two", "id": 2},
            {"media_id": "m2", "id": 3}
        ]},
        "media_metadata": {
            "m0": image_asset(server_uri, "m0"),
            "m1": {
                "status": "valid", "e": "Image", "m": "image/png",
                "s": {"u": format!("{server_uri}/prev/m1.png?width=640"), "x": 640, "y": 640}
            },
            "m2": {
                "status": "valid", "e": "AnimatedImage", "m": "image/png",
                "s": {
                    "u": format!("{server_uri}/prev/m2.png?width=500"),
                    "mp4": format!("{server_uri}/m2.mp4"),
                    "x": 500, "y": 500
                }
            }
        }
    })
}

fn image_post(server_uri: &str) -> Value {
    json!({
        "id": "i1",
        "name": "t3_i1",
        "title": "Direct image",
        "author": "poster",
        "subreddit": "testsub",
        "permalink": "/r/testsub/comments/i1/direct_image/",
        "url": format!("{server_uri}/i/image1.jpeg"),
        "domain": "i.redd.it",
        "post_hint": "image",
        "created_utc": 1700000000.0,
        "score": 50,
        "num_comments": 2,
        "is_gallery": false,
        "thumbnail": format!("{server_uri}/thumbs/image1.jpg"),
        "preview": {"images": [{"source": {
            "url": format!("{server_uri}/prev/image1.jpeg?s=1"),
            "width": 1080, "height": 1080
        }}]}
    })
}

fn video_post(server_uri: &str) -> Value {
    json!({
        "id": "v1",
        "name": "t3_v1",
        "title": "Redgifs clip",
        "author": "poster",
        "subreddit": "testsub",
        "permalink": "/r/testsub/comments/v1/redgifs_clip/",
        "url": "https://www.redgifs.com/watch/example",
        "domain": "redgifs.com",
        "post_hint": "rich:video",
        "created_utc": 1699900000.0,
        "score": 10,
        "num_comments": 0,
        "over_18": true,
        "is_video": true,
        "preview": {"images": [{"source": {
            "url": format!("{server_uri}/prev/video1.jpg"),
            "width": 720, "height": 1280
        }}]},
        "media": {"reddit_video": {
            "fallback_url": format!("{server_uri}/v/video1.mp4?source=fallback"),
            "width": 720, "height": 1280, "duration": 10.0
        }}
    })
}

fn page_one(server_uri: &str) -> Value {
    json!({
        "kind": "Listing",
        "data": {
            "after": "t3_g1",
            "children": [
                {"kind": "t3", "data": gallery_post(server_uri)},
                {"kind": "t3", "data": image_post(server_uri)},
                {"kind": "t3", "data": video_post(server_uri)},
            ]
        }
    })
}

fn page_two(server_uri: &str) -> Value {
    let mut p = image_post(server_uri);
    p["id"] = json!("i2");
    p["permalink"] = json!("/r/testsub/comments/i2/second/");
    p["title"] = json!("Second page image");
    p["created_utc"] = json!(1699800000.0);
    p["url"] = json!(format!("{server_uri}/i/image2.jpeg"));
    p["preview"]["images"][0]["source"]["url"] =
        json!(format!("{server_uri}/prev/image2.jpeg?s=2"));
    json!({
        "kind": "Listing",
        "data": {"after": null, "children": [{"kind": "t3", "data": p}]}
    })
}

/// A second subreddit, used for the multi-archive hub tests.
fn second_page(server_uri: &str) -> Value {
    json!({
        "kind": "Listing",
        "data": {"after": null, "children": [{"kind": "t3", "data": image_post(server_uri)}]}
    })
}

/// A tiny subreddit with a GIF and a JPEG post, used by the format-filter
/// tests. `broken_gif` adds a post whose GIF 404s (so only its JPEG preview
/// responds), which a GIF-only run must not save.
fn gifs_page(server_uri: &str, broken_gif: bool) -> Value {
    let post = |id: &str, original: &str, preview: &str| {
        json!({
            "id": id,
            "name": format!("t3_{id}"),
            "title": format!("Post {id}"),
            "author": "poster",
            "subreddit": "gifs",
            "permalink": format!("/r/gifs/comments/{id}/post/"),
            "url": format!("{server_uri}{original}"),
            "domain": "i.redd.it",
            "post_hint": "image",
            "created_utc": 1700200000.0,
            "thumbnail": format!("{server_uri}/thumbs/{id}.jpg"),
            "preview": {"images": [{"source": {
                "url": format!("{server_uri}{preview}"),
                "width": 200, "height": 200
            }}]}
        })
    };
    let mut children = vec![json!({
        "kind": "t3",
        "data": post("a1", "/i/animated.gif", "/prev/animated.jpg")
    })];
    if broken_gif {
        children.push(json!({
            "kind": "t3",
            "data": post("b1", "/i/broken.gif", "/prev/broken.jpg")
        }));
    }
    children.push(json!({
        "kind": "t3",
        "data": post("j1", "/i/still.jpg", "/prev/still.jpg")
    }));
    // reddit serves JPEG for `….png?format=pjpg` URLs; the stored file must be
    // named .jpg and the format filter must see it as jpg
    children.push(json!({
        "kind": "t3",
        "data": {
            "id": "p1",
            "name": "t3_p1",
            "title": "Profiled still",
            "author": "poster",
            "subreddit": "gifs",
            "permalink": "/r/gifs/comments/p1/profiled/",
            "url": format!("{server_uri}/x/thumb.png?format=pjpg&auto=webp&s=1"),
            "domain": "external-preview.redd.it",
            "post_hint": "image",
            "created_utc": 1700200000.0,
        }
    }));
    json!({"kind": "Listing", "data": {"after": null, "children": children}})
}

async fn mount_api(server: &MockServer, uri: &str) {
    Mock::given(method("GET"))
        .and(path("/r/testsub/about.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "t5",
            "data": {
                "display_name": "testsub",
                "title": "Test Sub",
                "public_description": "a test subreddit",
                "subscribers": 1234,
                "created_utc": 1600000000.0,
                "over18": true,
                "icon_img": format!("{uri}/art/icon.png?width=256"),
                "banner_background_image": format!("{uri}/art/banner.jpg")
            }
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/testsub/hot.json"))
        .and(query_param_is_missing("after"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_one(uri)))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/testsub/new.json"))
        .and(query_param("limit", "5"))
        .and(query_param_is_missing("after"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_one(uri)))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/testsub/new.json"))
        .and(query_param("after", "t3_g1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "Listing",
            "data": {"after": null, "children": []}
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/testsub/hot.json"))
        .and(query_param("after", "t3_g1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page_two(uri)))
        .mount(server)
        .await;

    // user listing (no about endpoint involved)
    let mut user_page = page_one(uri);
    user_page["data"]["after"] = json!(null);
    Mock::given(method("GET"))
        .and(path("/user/testuser/submitted.json"))
        .and(query_param_is_missing("after"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_page))
        .mount(server)
        .await;

    // second subreddit (multi-archive hub tests)
    Mock::given(method("GET"))
        .and(path("/r/second/about.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "t5",
            "data": {
                "display_name": "second",
                "title": "Second Sub",
                "subscribers": 10,
                "over18": false,
                "icon_img": format!("{uri}/art/icon.png")
            }
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/second/hot.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(second_page(uri)))
        .mount(server)
        .await;

    // a listing that requires sign-in
    Mock::given(method("GET"))
        .and(path("/r/forbidden/hot.json"))
        .respond_with(ResponseTemplate::new(403).set_body_string("blocked"))
        .mount(server)
        .await;

    for p in [
        "/art/icon.png",
        "/art/banner.jpg",
        "/m0.jpg",
        "/m1.png",
        "/m2.png",
        "/m2.mp4",
        "/prev/m0.jpg",
        "/prev/m1.png",
        "/prev/m2.png",
        "/prev/video1.jpg",
        "/i/image1.jpeg",
        "/prev/image1.jpeg",
        "/thumbs/image1.jpg",
        "/i/image2.jpeg",
        "/prev/image2.jpeg",
        "/v/video1.mp4",
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(format!("FAKE-{p}").into_bytes()),
            )
            .mount(server)
            .await;
    }
}

/// Mounts `/r/gifs/*` plus the media endpoints used by the format filter tests.
async fn mount_gifs_api(server: &MockServer, uri: &str, broken_gif: bool) {
    Mock::given(method("GET"))
        .and(path("/r/gifs/about.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "t5",
            "data": {"display_name": "gifs", "title": "Gifs", "subscribers": 5, "over18": false}
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/gifs/hot.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gifs_page(uri, broken_gif)))
        .mount(server)
        .await;

    for (p, mime) in [
        ("/i/animated.gif", "image/gif"),
        ("/prev/animated.jpg", "image/jpeg"),
        ("/i/still.jpg", "image/jpeg"),
        ("/prev/still.jpg", "image/jpeg"),
        ("/x/thumb.png", "image/jpeg"), // `?format=pjpg` is served as JPEG
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", mime)
                    .set_body_bytes(format!("FAKE-{p}").into_bytes()),
            )
            .mount(server)
            .await;
    }

    if broken_gif {
        Mock::given(method("GET"))
            .and(path("/i/broken.gif"))
            .respond_with(ResponseTemplate::new(404))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/prev/broken.jpg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/jpeg")
                    .set_body_bytes(b"FAKE-JPEG".to_vec()),
            )
            .mount(server)
            .await;
    }
}

/// A subreddit with one oversized PNG and one small JPEG, used by the
/// conversion/resize tests.
fn imaging_page(server_uri: &str) -> Value {
    let post = |id: &str, file: &str, w: i64, h: i64| {
        json!({
            "id": id,
            "name": format!("t3_{id}"),
            "title": format!("Post {id}"),
            "author": "poster",
            "subreddit": "imaging",
            "permalink": format!("/r/imaging/comments/{id}/post/"),
            "url": format!("{server_uri}{file}"),
            "domain": "i.redd.it",
            "post_hint": "image",
            "created_utc": 1700300000.0,
            "preview": {"images": [{"source": {
                "url": format!("{server_uri}/prev/{id}.jpg"),
                "width": w, "height": h
            }}]}
        })
    };
    json!({
        "kind": "Listing",
        "data": {
            "after": null,
            "children": [
                {"kind": "t3", "data": post("big1", "/i/big.png", 2000, 1000)},
                {"kind": "t3", "data": post("small1", "/i/small.jpg", 300, 200)}
            ]
        }
    })
}

fn png_image(w: u32, h: u32) -> Vec<u8> {
    let mut img = image::RgbaImage::new(w, h);
    for (x, _, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba([(x % 255) as u8, 40, 200, 255]);
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn jpeg_image(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::new_rgb8(w, h)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    out
}

/// Mounts `/r/imaging/*` plus real PNG/JPEG media.
async fn mount_imaging_api(server: &MockServer, uri: &str) {
    Mock::given(method("GET"))
        .and(path("/r/imaging/about.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "t5",
            "data": {"display_name": "imaging", "title": "Imaging", "subscribers": 5, "over18": false}
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/r/imaging/hot.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imaging_page(uri)))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/i/big.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(png_image(2000, 1000)),
        )
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/i/small.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .set_body_bytes(jpeg_image(300, 200)),
        )
        .mount(server)
        .await;
}

fn base_cfg(server_uri: &str, outdir: &std::path::Path) -> Config {
    Config {
        target: Target::subreddit("testsub"),
        cookies: None,
        user_agent: UA.into(),
        out_dir: outdir.to_path_buf(),
        posts: PostsMode::Snapshot,
        sort: Sort::Hot,
        time: reddit::TimeFilter::All,
        videos: false,
        formats: Vec::new(),
        min_size: None,
        max_size: None,
        convert: None,
        quality: 85,
        gallery_images: 0,
        since: None,
        skip_icon: false,
        no_raw: false,
        no_downloads: false,
        offline: false,
        base_url: server_uri.to_string(),
        media_base: server_uri.to_string(),
    }
}

fn outdir(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    dir.join(name)
}

fn read_json(dir: &std::path::Path, name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(outdir(dir, "r_testsub").join(name)).unwrap())
        .unwrap()
}

fn manifest(dir: &std::path::Path) -> Vec<ManifestItem> {
    serde_json::from_str(
        &std::fs::read_to_string(outdir(dir, "r_testsub").join("media_manifest.json")).unwrap(),
    )
    .unwrap()
}

async fn run_cfg(cfg: Config) -> reddit::Summary {
    reddit::run(cfg).await.unwrap()
}

/// Requests that are not listing/about JSON — i.e. media fetches.
async fn media_requests(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| !r.url.path().ends_with(".json"))
        .count()
}

#[tokio::test]
async fn default_snapshot_downloads_images_only() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let summary = run_cfg(base_cfg(&uri, dir.path())).await;
    assert_eq!(summary.posts, 3);
    // icon + banner + 3 gallery + 1 image post + 1 video preview = 7 images
    assert_eq!(summary.media_total, 7);
    assert_eq!(summary.media_failed, 0);

    let media = outdir(dir.path(), "r_testsub").join("media");
    for f in [
        "subreddit_icon.png",
        "subreddit_banner.jpg",
        "posts/g1_00.jpg",
        "posts/g1_01.png",
        "posts/g1_02.png",
        "posts/i1.jpeg",
        "posts/v1.jpg",
    ] {
        assert!(media.join(f).exists(), "missing {f}");
    }
    // images only: no video files
    assert!(!media.join("posts/g1_02.mp4").exists());
    assert!(!media.join("posts/v1.mp4").exists());

    let m = manifest(dir.path());
    let kinds: Vec<&str> = m.iter().map(|x| x.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "icon", "banner", "gallery", "gallery", "gallery", "image", "image"
        ]
    );
    // gallery images keep the original URL and the signed preview as fallback
    let first = &m[2];
    assert_eq!(first.url, format!("{uri}/m0.jpg"));
    assert_eq!(
        first.fallback.as_deref(),
        Some(format!("{uri}/prev/m0.jpg?width=800&format=pjpg&s=sig").as_str())
    );

    let posts = read_json(dir.path(), "testsub_posts.json");
    assert_eq!(posts["total"], 3);
    assert_eq!(posts["sort"], "hot");
    assert_eq!(posts["posts"][0]["gallery"].as_array().unwrap().len(), 3);
    assert_eq!(posts["posts"][0]["gallery"][1]["caption"], "two");
    assert_eq!(
        posts["posts"][1]["thumbnail"],
        format!("{uri}/thumbs/image1.jpg")
    );
    assert!(
        outdir(dir.path(), "r_testsub")
            .join("testsub_posts_raw.json")
            .exists()
    );
    assert!(
        outdir(dir.path(), "r_testsub")
            .join("testsub_about.json")
            .exists()
    );
}

#[tokio::test]
async fn pagination_cap_videos_and_gallery_cap() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.posts = PostsMode::All(0);
    cfg.videos = true;
    cfg.gallery_images = 2;
    let summary = run_cfg(cfg).await;
    assert_eq!(summary.posts, 4);

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs.iter()
            .filter(|r| r.url.path() == "/r/testsub/hot.json")
            .count(),
        2
    );

    let media = outdir(dir.path(), "r_testsub").join("media");
    assert!(media.join("posts/g1_00.jpg").exists());
    assert!(media.join("posts/g1_01.png").exists());
    assert!(!media.join("posts/g1_02.png").exists()); // gallery cap = 2
    assert!(!media.join("posts/g1_02.mp4").exists()); // capped item is skipped entirely
    assert!(media.join("posts/v1.mp4").exists()); // main video still downloaded
    assert!(media.join("posts/i2.jpeg").exists());
}

#[tokio::test]
async fn posts_five_fetches_only_five() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.sort = Sort::New;
    cfg.posts = PostsMode::All(5);
    let summary = run_cfg(cfg).await;
    assert_eq!(summary.posts, 3); // only 3 available in the mock

    let reqs = server.received_requests().await.unwrap();
    let new_reqs: Vec<_> = reqs
        .iter()
        .filter(|r| r.url.path() == "/r/testsub/new.json")
        .collect();
    assert_eq!(new_reqs.len(), 2); // page 1 was short, so pagination continued
    assert!(new_reqs[0].url.query().unwrap().contains("limit=5"));
    assert!(!new_reqs[0].url.query().unwrap().contains("after="));
}

#[tokio::test]
async fn since_filter_drops_old_posts() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.since = Some("2023-11-14".into()); // 1699920000: keeps g1 + i1, drops v1
    let summary = run_cfg(cfg).await;
    assert_eq!(summary.posts, 2);
    let posts = read_json(dir.path(), "testsub_posts.json");
    let ids: Vec<&str> = posts["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["g1", "i1"]);
}

#[tokio::test]
async fn cookies_and_user_agent_are_sent() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.cookies = Some("reddit_session=secret; csrf_token=tok".into());
    run_cfg(cfg).await;

    let reqs = server.received_requests().await.unwrap();
    let listing = reqs
        .iter()
        .find(|r| r.url.path() == "/r/testsub/hot.json")
        .unwrap();
    let cookie = listing.headers.get("cookie").unwrap().to_str().unwrap();
    assert!(cookie.contains("reddit_session=secret"));
    assert!(cookie.contains("csrf_token=tok"));
    assert!(cookie.contains("over18=1")); // NSFW opt-in added automatically
    assert_eq!(
        listing.headers.get("user-agent").unwrap().to_str().unwrap(),
        UA
    );

    // Media on non-Reddit origins must not receive the session.
    let media_req = reqs.iter().find(|r| r.url.path() == "/m0.jpg").unwrap();
    assert!(media_req.headers.get("cookie").is_none());
}

#[tokio::test]
async fn no_downloads_and_no_raw_keep_jsons_only() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.no_downloads = true;
    cfg.no_raw = true;
    let summary = run_cfg(cfg).await;
    assert_eq!(summary.media_total, 0);
    assert!(
        outdir(dir.path(), "r_testsub")
            .join("testsub_posts.json")
            .exists()
    );
    assert!(
        !outdir(dir.path(), "r_testsub")
            .join("testsub_posts_raw.json")
            .exists()
    );
    assert!(
        !outdir(dir.path(), "r_testsub")
            .join("media_manifest.json")
            .exists()
    );
    assert!(!outdir(dir.path(), "r_testsub").join("media").exists());
}

#[tokio::test]
async fn skip_icon_omits_subreddit_art() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.skip_icon = true;
    let summary = run_cfg(cfg).await;
    assert_eq!(summary.media_total, 5); // no icon/banner
    let m = manifest(dir.path());
    assert!(!m.iter().any(|x| x.kind == "icon" || x.kind == "banner"));
}

#[tokio::test]
async fn forbidden_listing_gives_helpful_error() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("forbidden");
    let err = reddit::run(cfg).await.unwrap_err().to_string();
    assert!(err.contains("403"), "unexpected error: {err}");
    assert!(err.contains("cookies"), "unexpected error: {err}");
}

#[tokio::test]
async fn user_target_uses_submitted_listing() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::user("testuser");
    let summary = run_cfg(cfg).await;
    assert_eq!(summary.posts, 3);

    let saved = outdir(dir.path(), "u_testuser").join("testuser_posts.json");
    assert!(saved.exists());
    // no about request for users
    let reqs = server.received_requests().await.unwrap();
    assert!(!reqs.iter().any(|r| r.url.path().contains("about")));
}

#[tokio::test]
async fn offline_viewer_renders_local_media() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.offline = true;
    run_cfg(cfg).await;

    let html = std::fs::read_to_string(outdir(dir.path(), "r_testsub").join("index.html")).unwrap();
    assert!(html.contains("r/testsub"));
    assert!(html.contains("Test Sub"));
    assert!(html.contains("a test subreddit"));
    assert!(html.contains("media/posts/g1_00.jpg"));
    assert!(html.contains("media/posts/i1.jpeg"));
    assert!(html.contains("media/subreddit_icon.png"));
    assert!(html.contains("Jane gallery"));
    assert!(!html.contains("__DATA__"));
}

#[tokio::test]
async fn rerun_uses_the_cache_instead_of_downloading_again() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let first = run_cfg(base_cfg(&uri, dir.path())).await;
    assert_eq!(first.posts, 3);
    assert_eq!(first.posts_new, 3);
    assert_eq!(first.media_downloaded, 7);
    assert_eq!(first.media_cached, 0);
    let requests_after_first = media_requests(&server).await;
    assert_eq!(requests_after_first, 7);

    let second = run_cfg(base_cfg(&uri, dir.path())).await;
    assert_eq!(second.posts, 3);
    assert_eq!(second.posts_new, 0);
    assert_eq!(second.media_downloaded, 0);
    assert_eq!(second.media_cached, 7);
    assert_eq!(second.media_failed, 0);
    assert_eq!(
        media_requests(&server).await,
        requests_after_first,
        "cached media must not be requested again"
    );
}

#[tokio::test]
async fn empty_files_are_not_treated_as_cached() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    // populate the archive, then corrupt one file as an interrupted run would
    run_cfg(base_cfg(&uri, dir.path())).await;
    let media = outdir(dir.path(), "r_testsub").join("media/posts");
    std::fs::write(media.join("i1.jpeg"), b"").unwrap();

    let summary = run_cfg(base_cfg(&uri, dir.path())).await;
    assert_eq!(summary.media_cached, 6);
    assert_eq!(summary.media_downloaded, 1);
    assert_eq!(summary.media_failed, 0);
    assert!(std::fs::metadata(media.join("i1.jpeg")).unwrap().len() > 0);
    assert!(!media.join("i1.jpeg.part").exists());
}

#[tokio::test]
async fn rerun_merges_posts_instead_of_replacing_them() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    // first run archives a single post...
    let mut cfg = base_cfg(&uri, dir.path());
    cfg.posts = PostsMode::All(1);
    let first = run_cfg(cfg).await;
    assert_eq!(first.posts, 1);

    // ...the second run adds the other two without dropping the first
    let second = run_cfg(base_cfg(&uri, dir.path())).await;
    assert_eq!(second.posts, 3);
    assert_eq!(second.posts_new, 2);

    let posts = read_json(dir.path(), "testsub_posts.json");
    assert_eq!(posts["total"], 3);
    assert_eq!(posts["posts"].as_array().unwrap().len(), 3);
    let ids: Vec<&str> = posts["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["g1", "i1", "v1"]);
}

#[tokio::test]
async fn offline_multi_target_writes_hub_and_backlinks() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut first = base_cfg(&uri, dir.path());
    first.offline = true;
    run_cfg(first).await;

    let mut second = base_cfg(&uri, dir.path());
    second.target = Target::subreddit("second");
    second.offline = true;
    run_cfg(second).await;

    let hub = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
    assert!(hub.contains("\"dir\":\"r_testsub\""));
    assert!(hub.contains("\"dir\":\"r_second\""));
    assert!(hub.contains("\"viewer\":true"));
    assert!(!hub.contains("__DATA__"));

    let inner =
        std::fs::read_to_string(outdir(dir.path(), "r_testsub").join("index.html")).unwrap();
    assert!(inner.contains("\"hub\":true"));
}

#[tokio::test]
async fn formats_filter_downloads_only_matching_media() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    mount_gifs_api(&server, &uri, false).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("gifs");
    cfg.formats = vec![MediaFormat::Gif];
    let summary = run_cfg(cfg).await;

    // the JSON keeps every post; only the GIF is downloaded
    assert_eq!(summary.posts, 3);
    assert_eq!(summary.media_total, 1);
    assert_eq!(summary.media_downloaded, 1);
    assert_eq!(summary.media_failed, 0);

    let archive = outdir(dir.path(), "r_gifs");
    assert!(archive.join("media/posts/a1.gif").exists());
    assert!(!archive.join("media/posts/j1.jpg").exists());
    assert!(!archive.join("media/posts/p1.jpg").exists());

    let posts: Value =
        serde_json::from_str(&std::fs::read_to_string(archive.join("gifs_posts.json")).unwrap())
            .unwrap();
    assert_eq!(posts["total"], 3);
    assert_eq!(posts["posts"].as_array().unwrap().len(), 3);

    let reqs = server.received_requests().await.unwrap();
    let media: Vec<&str> = reqs
        .iter()
        .filter(|r| !r.url.path().ends_with(".json"))
        .map(|r| r.url.path())
        .collect();
    assert_eq!(
        media,
        vec!["/i/animated.gif"],
        "only the gif may be fetched"
    );
}

#[tokio::test]
async fn formats_reject_mismatched_fallback_content() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    mount_gifs_api(&server, &uri, true).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("gifs");
    cfg.formats = vec![MediaFormat::Gif];
    let summary = run_cfg(cfg).await;

    assert_eq!(summary.posts, 4);
    assert_eq!(summary.media_total, 2);
    assert_eq!(summary.media_downloaded, 1);
    assert_eq!(summary.media_failed, 1);

    let media = outdir(dir.path(), "r_gifs").join("media");
    assert!(media.join("posts/a1.gif").exists());
    assert!(
        !media.join("posts/b1.gif").exists(),
        "a JPEG preview must not be stored as .gif"
    );
}

#[tokio::test]
async fn format_query_decides_the_stored_extension() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    mount_gifs_api(&server, &uri, false).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("gifs");
    cfg.formats = vec![MediaFormat::Jpg];
    let summary = run_cfg(cfg).await;

    // still.jpg and `….png?format=pjpg` are both JPEG
    assert_eq!(summary.media_total, 2);
    assert_eq!(summary.media_downloaded, 2);
    assert_eq!(summary.media_failed, 0);

    let media = outdir(dir.path(), "r_gifs").join("media/posts");
    assert!(media.join("j1.jpg").exists());
    assert!(media.join("p1.jpg").exists());
    assert!(!media.join("p1.png").exists());
}

#[tokio::test]
async fn formats_video_implies_videos() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.formats = vec![MediaFormat::Mp4]; // --videos not passed
    let summary = run_cfg(cfg).await;

    assert_eq!(summary.media_total, 2); // gallery animation + reddit video
    assert_eq!(summary.media_downloaded, 2);
    assert_eq!(summary.media_failed, 0);

    let media = outdir(dir.path(), "r_testsub").join("media");
    assert!(media.join("posts/g1_02.mp4").exists());
    assert!(media.join("posts/v1.mp4").exists());
    assert!(!media.join("posts/i1.jpeg").exists());
}

#[tokio::test]
async fn min_size_skips_small_stills() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.min_size = Some((768, 1024));
    let summary = run_cfg(cfg).await;

    // icon + banner + m0 (800x1200) + i1 (1080x1080) survive;
    // m1 (640x640), m2 (500x500) and the v1 preview (720x1280) are too small
    assert_eq!(summary.media_total, 4);
    assert_eq!(summary.media_downloaded, 4);
    assert_eq!(summary.media_skipped, 3);
    assert_eq!(summary.media_failed, 0);

    let media = outdir(dir.path(), "r_testsub").join("media");
    assert!(media.join("posts/g1_00.jpg").exists());
    assert!(media.join("posts/i1.jpeg").exists());
    assert!(!media.join("posts/g1_01.png").exists());
    assert!(!media.join("posts/g1_02.png").exists());
    assert!(!media.join("posts/v1.jpg").exists());
}

#[tokio::test]
async fn convert_and_resize_rewrite_new_downloads() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    mount_imaging_api(&server, &uri).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("imaging");
    cfg.convert = Some(MediaFormat::Jpg);
    cfg.max_size = Some((1344, 1792));
    let summary = run_cfg(cfg).await;

    assert_eq!(summary.media_total, 2);
    assert_eq!(summary.media_downloaded, 2);
    assert_eq!(summary.media_converted, 1);
    assert_eq!(summary.media_resized, 1);
    assert_eq!(summary.media_failed, 0);

    let media = outdir(dir.path(), "r_imaging").join("media");
    assert!(!media.join("posts/big1.png").exists());
    let big =
        image::load_from_memory(&std::fs::read(media.join("posts/big1.jpg")).unwrap()).unwrap();
    assert_eq!((big.width(), big.height()), (1344, 672));

    // already JPEG and within the box: stored byte-for-byte (no re-encode)
    assert_eq!(
        std::fs::read(media.join("posts/small1.jpg")).unwrap(),
        jpeg_image(300, 200)
    );
}

#[tokio::test]
async fn convert_keeps_gifs_untouched() {
    let server = MockServer::start().await;
    let uri = server.uri();
    mount_api(&server, &uri).await;
    mount_gifs_api(&server, &uri, false).await;
    let dir = tempdir().unwrap();

    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("gifs");
    cfg.formats = vec![MediaFormat::Gif];
    cfg.convert = Some(MediaFormat::Jpg);
    cfg.max_size = Some((10, 10));
    let summary = run_cfg(cfg).await;

    assert_eq!(summary.media_total, 1);
    assert_eq!(summary.media_downloaded, 1);
    assert_eq!(summary.media_converted, 0);
    assert_eq!(summary.media_resized, 0);

    let media = outdir(dir.path(), "r_gifs").join("media");
    assert!(media.join("posts/a1.gif").exists());
    assert!(!media.join("posts/a1.jpg").exists());
    assert_eq!(
        std::fs::read(media.join("posts/a1.gif")).unwrap(),
        b"FAKE-/i/animated.gif"
    );
}

#[tokio::test]
async fn invalid_target_is_rejected_before_network() {
    let server = MockServer::start().await;
    let uri = server.uri();
    let dir = tempdir().unwrap();
    let mut cfg = base_cfg(&uri, dir.path());
    cfg.target = Target::subreddit("bad name!");
    assert!(reddit::run(cfg).await.is_err());
}

#[test]
fn target_parsing_matches_cli_docs() {
    let t = reddit::parse_target("https://www.reddit.com/r/funny").unwrap();
    assert_eq!(t.target, Target::subreddit("funny"));
    let t = reddit::parse_target("u/spez").unwrap();
    assert_eq!(t.target, Target::user("spez"));
}

#[test]
fn cookie_helpers_are_available_to_library_users() {
    let c = clean::parse_cookies("a=1; b=2");
    assert_eq!(c.len(), 2);
}
