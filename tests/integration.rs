//! End-to-end runs against a mocked Reddit API and mocked media hosts.

use reddit::{
    AssetKind, Config, ConvertFormat, Cookies, ManifestItem, MediaFormat, PostsMode, Size, Sort,
    Summary, Target,
};
use serde_json::{Value, json};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gallery_post(uri: &str) -> Value {
    json!({
        "id": "g1",
        "name": "t3_g1",
        "title": "Jane gallery",
        "author": "poster",
        "subreddit": "testsub",
        "permalink": "/r/testsub/comments/g1/jane_gallery/",
        "url": "https://www.reddit.com/gallery/g1",
        "created_utc": 1700100000.0,
        "score": 100,
        "num_comments": 4,
        "over_18": true,
        "is_gallery": true,
        "gallery_data": {"items": [
            {"media_id": "m0", "id": 1},
            {"media_id": "m1", "caption": "two", "id": 2},
            {"media_id": "m2", "id": 3}
        ]},
        "media_metadata": {
            "m0": {
                "status": "valid", "e": "Image", "m": "image/jpg",
                "s": {"u": format!("{uri}/prev/m0.jpg?width=800&format=pjpg&s=sig"), "x": 800, "y": 1200}
            },
            "m1": {
                "status": "valid", "e": "Image", "m": "image/png",
                "s": {"u": format!("{uri}/prev/m1.png?width=640"), "x": 640, "y": 640}
            },
            "m2": {
                "status": "valid", "e": "AnimatedImage", "m": "image/png",
                "s": {"u": format!("{uri}/prev/m2.png?width=500"), "mp4": format!("{uri}/m2.mp4"), "x": 500, "y": 500}
            }
        }
    })
}

fn image_post(uri: &str, id: &str, created: f64) -> Value {
    json!({
        "id": id,
        "name": format!("t3_{id}"),
        "title": format!("Image {id}"),
        "author": "poster",
        "subreddit": "testsub",
        "permalink": format!("/r/testsub/comments/{id}/image/"),
        "url": format!("{uri}/i/{id}.jpeg"),
        "domain": "i.redd.it",
        "post_hint": "image",
        "created_utc": created,
        "thumbnail": format!("{uri}/thumbs/{id}.jpg"),
        "preview": {"images": [{"source": {
            "url": format!("{uri}/prev/{id}.jpeg?s=1"),
            "width": 1080, "height": 1080
        }}]}
    })
}

fn video_post(uri: &str) -> Value {
    json!({
        "id": "v1",
        "name": "t3_v1",
        "title": "Clip",
        "author": "poster",
        "subreddit": "testsub",
        "permalink": "/r/testsub/comments/v1/clip/",
        "url": "https://v.redd.it/v1",
        "post_hint": "hosted:video",
        "created_utc": 1699900000.0,
        "is_video": true,
        "preview": {"images": [{"source": {"url": format!("{uri}/prev/v1.jpg"), "width": 720, "height": 1280}}]},
        "media": {"reddit_video": {
            "fallback_url": format!("{uri}/v/v1.mp4?source=fallback"),
            "width": 720, "height": 1280, "duration": 10.0
        }}
    })
}

fn listing(after: Option<&str>, posts: Vec<Value>) -> Value {
    let children: Vec<Value> = posts
        .into_iter()
        .map(|data| json!({"kind": "t3", "data": data}))
        .collect();
    json!({"kind": "Listing", "data": {"after": after, "children": children}})
}

fn page_one(uri: &str) -> Value {
    listing(
        Some("t3_g1"),
        vec![
            gallery_post(uri),
            image_post(uri, "i1", 1700000000.0),
            video_post(uri),
        ],
    )
}

async fn mount_json(server: &MockServer, at: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_file(server: &MockServer, at: &str, content_type: Option<&str>, body: Vec<u8>) {
    let mut response = ResponseTemplate::new(200).set_body_bytes(body);
    if let Some(content_type) = content_type {
        response = response.insert_header("content-type", content_type);
    }
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(response)
        .mount(server)
        .await;
}

/// `r/testsub` (two pages), `r/second`, `u/testuser`, `r/forbidden` and media.
async fn mount_api(server: &MockServer) {
    let uri = server.uri();
    mount_json(
        server,
        "/r/testsub/about.json",
        json!({"kind": "t5", "data": {
            "display_name": "testsub",
            "title": "Test Sub",
            "public_description": "a test subreddit",
            "subscribers": 1234,
            "over18": true,
            "icon_img": format!("{uri}/art/icon.png?width=256"),
            "banner_background_image": format!("{uri}/art/banner.jpg")
        }}),
    )
    .await;
    for sort in ["hot", "new"] {
        Mock::given(method("GET"))
            .and(path(format!("/r/testsub/{sort}.json")))
            .and(query_param_is_missing("after"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_one(&uri)))
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/r/testsub/hot.json"))
        .and(query_param("after", "t3_g1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(listing(None, vec![image_post(&uri, "i2", 1699800000.0)])),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r/testsub/new.json"))
        .and(query_param("after", "t3_g1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(listing(None, vec![])))
        .mount(server)
        .await;

    let mut user_page = page_one(&uri);
    user_page["data"]["after"] = Value::Null;
    mount_json(server, "/user/testuser/submitted.json", user_page).await;

    mount_json(
        server,
        "/r/second/about.json",
        json!({"kind": "t5", "data": {"display_name": "second", "title": "Second Sub", "icon_img": format!("{uri}/art/icon.png")}}),
    )
    .await;
    mount_json(
        server,
        "/r/second/hot.json",
        listing(None, vec![image_post(&uri, "s1", 1700000000.0)]),
    )
    .await;

    Mock::given(method("GET"))
        .and(path("/r/forbidden/hot.json"))
        .respond_with(ResponseTemplate::new(403).set_body_string("blocked"))
        .mount(server)
        .await;

    for at in [
        "/art/icon.png",
        "/art/banner.jpg",
        "/m0.jpg",
        "/m1.png",
        "/m2.png",
        "/m2.mp4",
        "/prev/v1.jpg",
        "/i/i1.jpeg",
        "/i/i2.jpeg",
        "/i/s1.jpeg",
        "/v/v1.mp4",
    ] {
        mount_file(server, at, None, format!("FAKE-{at}").into_bytes()).await;
    }
}

/// `r/gifs`: a GIF, a JPEG and a resizer URL served as JPEG. With
/// `broken_gif`, a GIF that 404s and whose fallback is a JPEG preview.
async fn mount_gifs(server: &MockServer, broken_gif: bool) {
    let uri = server.uri();
    let post = |id: &str, original: &str, preview: Option<&str>| {
        let mut post = json!({
            "id": id,
            "title": format!("Post {id}"),
            "permalink": format!("/r/gifs/comments/{id}/post/"),
            "url": format!("{uri}{original}"),
            "created_utc": 1700200000.0,
        });
        if let Some(preview) = preview {
            post["preview"] = json!({"images": [{"source": {"url": format!("{uri}{preview}"), "width": 200, "height": 200}}]});
        }
        post
    };
    let mut posts = vec![post("a1", "/i/animated.gif", Some("/prev/animated.jpg"))];
    if broken_gif {
        posts.push(post("b1", "/i/broken.gif", Some("/prev/broken.jpg")));
    }
    posts.push(post("j1", "/i/still.jpg", Some("/prev/still.jpg")));
    // Reddit's resizer serves JPEG for `….png?format=pjpg`.
    posts.push(post("p1", "/x/thumb.png?format=pjpg&auto=webp&s=1", None));
    mount_json(
        server,
        "/r/gifs/about.json",
        json!({"data": {"display_name": "gifs"}}),
    )
    .await;
    mount_json(server, "/r/gifs/hot.json", listing(None, posts)).await;

    for (at, mime) in [
        ("/i/animated.gif", "image/gif"),
        ("/prev/animated.jpg", "image/jpeg"),
        ("/i/still.jpg", "image/jpeg"),
        ("/prev/still.jpg", "image/jpeg"),
        ("/x/thumb.png", "image/jpeg"),
        ("/prev/broken.jpg", "image/jpeg"),
    ] {
        mount_file(server, at, Some(mime), format!("FAKE-{at}").into_bytes()).await;
    }
    Mock::given(method("GET"))
        .and(path("/i/broken.gif"))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let image =
        image::RgbaImage::from_fn(w, h, |x, _| image::Rgba([(x % 255) as u8, 40, 200, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::new_rgb8(w, h)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    out
}

/// `r/imaging`: an oversized PNG and a small JPEG (real image data).
async fn mount_imaging(server: &MockServer) {
    let uri = server.uri();
    let post = |id: &str, file: &str, w: u32, h: u32| {
        json!({
            "id": id,
            "title": id,
            "permalink": format!("/r/imaging/comments/{id}/"),
            "url": format!("{uri}{file}"),
            "created_utc": 1700300000.0,
            "preview": {"images": [{"source": {"url": format!("{uri}/prev/{id}.jpg"), "width": w, "height": h}}]}
        })
    };
    mount_json(
        server,
        "/r/imaging/about.json",
        json!({"data": {"display_name": "imaging"}}),
    )
    .await;
    mount_json(
        server,
        "/r/imaging/hot.json",
        listing(
            None,
            vec![
                post("big1", "/i/big.png", 2000, 1000),
                post("small1", "/i/small.jpg", 300, 200),
            ],
        ),
    )
    .await;
    mount_file(server, "/i/big.png", Some("image/png"), png(2000, 1000)).await;
    mount_file(server, "/i/small.jpg", Some("image/jpeg"), jpeg(300, 200)).await;
}

/// A config for `name` against the mock server, without pacing delays.
fn config(server: &MockServer, out: &Path, target: Target) -> Config {
    let mut config = Config::new(target);
    config.out_dir = out.to_path_buf();
    config.api_base = server.uri();
    config.media_base = server.uri();
    config.page_delay = Duration::ZERO;
    config
}

fn testsub(server: &MockServer, out: &Path) -> Config {
    config(server, out, Target::subreddit("testsub").unwrap())
}

async fn run(config: &Config) -> Summary {
    reddit::run(config).await.unwrap()
}

fn read_json(path: PathBuf) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn manifest(archive: &Path) -> Vec<ManifestItem> {
    serde_json::from_slice(&std::fs::read(archive.join("manifest.json")).unwrap()).unwrap()
}

async fn requests_to(server: &MockServer, filter: impl Fn(&str) -> bool) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_owned())
        .filter(|p| filter(p))
        .collect()
}

async fn media_requests(server: &MockServer) -> usize {
    requests_to(server, |p| !p.ends_with(".json")).await.len()
}

fn assert_files(dir: &Path, present: &[&str], absent: &[&str]) {
    for file in present {
        assert!(dir.join(file).is_file(), "missing {file}");
    }
    for file in absent {
        assert!(!dir.join(file).exists(), "unexpected {file}");
    }
}

#[tokio::test]
async fn default_run_saves_the_first_page_and_its_images() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let summary = run(&testsub(&server, out.path())).await;
    assert_eq!((summary.posts, summary.posts_new), (3, 3));
    // icon, banner, 3 gallery images, 1 image post, 1 video preview
    assert_eq!(summary.media.total, 7);
    assert_eq!(summary.media.downloaded, 7);
    assert_eq!(summary.media.failed, 0);

    let archive = out.path().join("r_testsub");
    assert_files(
        &archive,
        &[
            "posts.json",
            "about.json",
            "raw.json",
            "manifest.json",
            "media/subreddit_icon.png",
            "media/subreddit_banner.jpg",
            "media/posts/g1_00.jpg",
            "media/posts/g1_01.png",
            "media/posts/g1_02.png",
            "media/posts/i1.jpg",
            "media/posts/v1.jpg",
        ],
        &["index.html", "media/posts/g1_02.mp4", "media/posts/v1.mp4"],
    );

    use AssetKind::*;
    let items = manifest(&archive);
    let kinds: Vec<_> = items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [Icon, Banner, Gallery, Gallery, Gallery, Image, Image]
    );
    assert_eq!(items[2].url, format!("{}/m0.jpg", server.uri()));
    assert_eq!(
        items[2].fallback,
        Some(format!(
            "{}/prev/m0.jpg?width=800&format=pjpg&s=sig",
            server.uri()
        ))
    );

    let posts = read_json(archive.join("posts.json"));
    assert_eq!(posts["target"], "r/testsub");
    assert_eq!(posts["sort"], "hot");
    assert_eq!(posts["posts"].as_array().unwrap().len(), 3);
    assert_eq!(posts["posts"][0]["gallery"][1]["caption"], "two");
    assert_eq!(posts["posts"][0]["gallery"][2]["kind"], "animated");
    assert_eq!(
        read_json(archive.join("about.json"))["about"]["title"],
        "Test Sub"
    );
}

#[tokio::test]
async fn pagination_videos_and_gallery_limit() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.posts = PostsMode::All;
    config.media.videos = true;
    config.media.gallery_limit = Some(2);
    assert_eq!(run(&config).await.posts, 4);
    assert_eq!(
        requests_to(&server, |p| p == "/r/testsub/hot.json")
            .await
            .len(),
        2
    );

    assert_files(
        &out.path().join("r_testsub/media/posts"),
        &["g1_00.jpg", "g1_01.png", "v1.mp4", "i2.jpg"],
        &["g1_02.png", "g1_02.mp4"],
    );
}

#[tokio::test]
async fn post_limits_set_the_page_size_and_continue_short_pages() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.sort = Sort::New;
    config.posts = PostsMode::Limit(NonZeroU64::new(5).unwrap());
    assert_eq!(run(&config).await.posts, 3);

    let requests = server.received_requests().await.unwrap();
    let pages: Vec<_> = requests
        .iter()
        .filter(|r| r.url.path() == "/r/testsub/new.json")
        .collect();
    assert_eq!(pages.len(), 2, "a short first page continues to the cursor");
    let first = pages[0].url.query().unwrap();
    assert!(
        first.contains("limit=5") && !first.contains("after="),
        "{first}"
    );
}

#[tokio::test]
async fn since_drops_older_fetched_posts() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.since = chrono::NaiveDate::from_ymd_opt(2023, 11, 14); // 1699920000
    assert_eq!(run(&config).await.posts, 2);
    let posts = read_json(out.path().join("r_testsub/posts.json"));
    let ids: Vec<_> = posts["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].clone())
        .collect();
    assert_eq!(ids, [json!("g1"), json!("i1")]);
}

#[tokio::test]
async fn cookies_and_user_agents_are_sent_to_reddit_only() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.cookies = Cookies::from_header("reddit_session=secret; csrf_token=tok").unwrap();
    config.user_agent = Some("Mozilla/5.0 (test)".into());
    run(&config).await;

    let requests = server.received_requests().await.unwrap();
    let listing = requests
        .iter()
        .find(|r| r.url.path() == "/r/testsub/hot.json")
        .unwrap();
    let cookie = listing.headers.get("cookie").unwrap().to_str().unwrap();
    assert!(cookie.contains("reddit_session=secret") && cookie.contains("csrf_token=tok"));
    assert!(cookie.contains("over18=1"), "NSFW opt-in is added");
    assert_eq!(
        listing.headers.get("user-agent").unwrap(),
        "Mozilla/5.0 (test)"
    );

    let media = requests.iter().find(|r| r.url.path() == "/m0.jpg").unwrap();
    assert!(
        media.headers.get("cookie").is_none(),
        "no session for non-Reddit hosts"
    );
}

#[tokio::test]
async fn default_user_agent_matches_the_emulated_browser() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();
    run(&testsub(&server, out.path())).await;

    let requests = server.received_requests().await.unwrap();
    let ua = requests[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ua.contains("Chrome/149"), "{ua}");
}

#[tokio::test]
async fn json_only_runs_skip_raw_dumps_and_media() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.download = false;
    config.save_raw = false;
    assert_eq!(run(&config).await.media.total, 0);
    assert_files(
        &out.path().join("r_testsub"),
        &["posts.json", "about.json"],
        &["raw.json", "manifest.json", "media"],
    );
}

#[tokio::test]
async fn subreddit_art_can_be_skipped() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.media.subreddit_art = false;
    assert_eq!(run(&config).await.media.total, 5);
    let items = manifest(&out.path().join("r_testsub"));
    assert!(items.iter().all(|i| i.kind.is_post_still()));
}

#[tokio::test]
async fn forbidden_listings_explain_and_leave_no_archive() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let config = config(&server, out.path(), Target::subreddit("forbidden").unwrap());
    let error = format!("{:#}", reddit::run(&config).await.unwrap_err());
    assert!(
        error.contains("403") && error.contains("--cookies"),
        "{error}"
    );
    assert!(!out.path().join("r_forbidden").exists());
}

#[tokio::test]
async fn rate_limited_listings_wait_and_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/busy/hot.json"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    mount_json(
        &server,
        "/r/busy/hot.json",
        listing(None, vec![json!({"id": "x1", "title": "ok"})]),
    )
    .await;
    let out = tempdir().unwrap();

    let mut config = config(&server, out.path(), Target::subreddit("busy").unwrap());
    config.download = false;
    assert_eq!(run(&config).await.posts, 1);
    assert_eq!(
        requests_to(&server, |p| p == "/r/busy/hot.json")
            .await
            .len(),
        2
    );
}

#[tokio::test]
async fn user_targets_use_the_submitted_listing() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let config = config(&server, out.path(), Target::user("testuser").unwrap());
    assert_eq!(run(&config).await.posts, 3);
    assert!(out.path().join("u_testuser/posts.json").is_file());
    assert!(
        requests_to(&server, |p| p.contains("about"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn offline_viewers_and_hub_link_every_archive() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    for target in ["testsub", "second"] {
        let mut config = config(&server, out.path(), Target::subreddit(target).unwrap());
        config.offline = true;
        run(&config).await;
    }

    let viewer = std::fs::read_to_string(out.path().join("r_testsub/index.html")).unwrap();
    for expected in [
        "Test Sub",
        "a test subreddit",
        "Jane gallery",
        "media/posts/g1_00.jpg",
        "media/posts/i1.jpg",
        "media/subreddit_icon.png",
        "\"hub\":true",
    ] {
        assert!(viewer.contains(expected), "viewer is missing {expected}");
    }
    let hub = std::fs::read_to_string(out.path().join("index.html")).unwrap();
    assert!(hub.contains("\"dir\":\"r_testsub\"") && hub.contains("\"dir\":\"r_second\""));
    assert!(!hub.contains("\"viewer\":false"));
}

#[tokio::test]
async fn reruns_use_cached_media_and_refetch_empty_files() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();
    let config = testsub(&server, out.path());

    let first = run(&config).await;
    assert_eq!((first.media.downloaded, first.media.cached), (7, 0));
    let fetched = media_requests(&server).await;
    assert_eq!(fetched, 7);

    let second = run(&config).await;
    assert_eq!(second.posts_new, 0);
    assert_eq!((second.media.downloaded, second.media.cached), (0, 7));
    assert_eq!(
        media_requests(&server).await,
        fetched,
        "cached media is not requested"
    );

    // an interrupted earlier run may leave an empty file behind
    let image = out.path().join("r_testsub/media/posts/i1.jpg");
    std::fs::write(&image, b"").unwrap();
    let third = run(&config).await;
    assert_eq!((third.media.downloaded, third.media.cached), (1, 6));
    assert!(std::fs::metadata(&image).unwrap().len() > 0);
    assert!(
        !out.path()
            .join("r_testsub/media/posts/i1.jpg.part")
            .exists()
    );
}

#[tokio::test]
async fn reruns_merge_posts_instead_of_replacing_them() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.posts = PostsMode::Limit(NonZeroU64::new(1).unwrap());
    assert_eq!(run(&config).await.posts, 1);

    config.posts = PostsMode::FirstPage;
    let second = run(&config).await;
    assert_eq!((second.posts, second.posts_new), (3, 2));
    let posts = read_json(out.path().join("r_testsub/posts.json"));
    let ids: Vec<_> = posts["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].clone())
        .collect();
    assert_eq!(ids, [json!("g1"), json!("i1"), json!("v1")]);
}

#[tokio::test]
async fn corrupt_archives_are_never_overwritten() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();
    let archive = out.path().join("r_testsub");
    std::fs::create_dir_all(&archive).unwrap();
    std::fs::write(archive.join("posts.json"), b"{truncated").unwrap();

    let error = format!(
        "{:#}",
        reddit::run(&testsub(&server, out.path()))
            .await
            .unwrap_err()
    );
    assert!(error.contains("posts.json"), "{error}");
    assert_eq!(
        std::fs::read(archive.join("posts.json")).unwrap(),
        b"{truncated"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn format_selection_downloads_only_matching_media() {
    let server = MockServer::start().await;
    mount_gifs(&server, false).await;
    let out = tempdir().unwrap();

    let mut config = config(&server, out.path(), Target::subreddit("gifs").unwrap());
    config.media.formats = vec![MediaFormat::Gif];
    let summary = run(&config).await;
    assert_eq!(summary.posts, 3, "JSON keeps every post");
    assert_eq!((summary.media.total, summary.media.downloaded), (1, 1));
    assert_eq!(
        requests_to(&server, |p| !p.ends_with(".json")).await,
        ["/i/animated.gif"]
    );
}

#[tokio::test]
async fn mismatched_fallback_content_is_rejected() {
    let server = MockServer::start().await;
    mount_gifs(&server, true).await;
    let out = tempdir().unwrap();

    let mut config = config(&server, out.path(), Target::subreddit("gifs").unwrap());
    config.media.formats = vec![MediaFormat::Gif];
    let summary = run(&config).await;
    assert_eq!(summary.media.total, 2);
    assert_eq!((summary.media.downloaded, summary.media.failed), (1, 1));
    assert_files(
        &out.path().join("r_gifs/media/posts"),
        &["a1.gif"],
        &["b1.gif"],
    );
    // 404 is permanent: one request, then the fallback
    assert_eq!(
        requests_to(&server, |p| p == "/i/broken.gif").await.len(),
        1
    );
    assert_eq!(
        requests_to(&server, |p| p == "/prev/broken.jpg")
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn resizer_format_decides_the_stored_extension() {
    let server = MockServer::start().await;
    mount_gifs(&server, false).await;
    let out = tempdir().unwrap();

    let mut config = config(&server, out.path(), Target::subreddit("gifs").unwrap());
    config.media.formats = vec![MediaFormat::Jpg];
    let summary = run(&config).await;
    assert_eq!((summary.media.total, summary.media.downloaded), (2, 2));
    assert_files(
        &out.path().join("r_gifs/media/posts"),
        &["j1.jpg", "p1.jpg"],
        &["p1.png", "a1.gif"],
    );
}

#[tokio::test]
async fn selecting_a_video_format_implies_videos() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.media.formats = vec![MediaFormat::Mp4];
    let summary = run(&config).await;
    assert_eq!((summary.media.total, summary.media.downloaded), (2, 2));
    assert_files(
        &out.path().join("r_testsub/media/posts"),
        &["g1_02.mp4", "v1.mp4"],
        &["i1.jpg"],
    );
}

#[tokio::test]
async fn min_size_skips_small_stills() {
    let server = MockServer::start().await;
    mount_api(&server).await;
    let out = tempdir().unwrap();

    let mut config = testsub(&server, out.path());
    config.media.min_size = Size::new(768, 1024);
    let summary = run(&config).await;
    // art (no dimensions), m0 (800x1200) and i1 (1080x1080) remain
    assert_eq!((summary.media.total, summary.media.skipped), (4, 3));
    assert_eq!(summary.media.downloaded, 4);
    assert_files(
        &out.path().join("r_testsub/media/posts"),
        &["g1_00.jpg", "i1.jpg"],
        &["g1_01.png", "g1_02.png", "v1.jpg"],
    );
}

#[tokio::test]
async fn conversion_and_resizing_rewrite_new_downloads() {
    let server = MockServer::start().await;
    mount_imaging(&server).await;
    let out = tempdir().unwrap();

    let mut config = config(&server, out.path(), Target::subreddit("imaging").unwrap());
    config.transforms.convert = Some(ConvertFormat::Jpg);
    config.transforms.max_size = Size::new(1344, 1792);
    let media = run(&config).await.media;
    assert_eq!(
        (
            media.downloaded,
            media.converted,
            media.resized,
            media.failed
        ),
        (2, 1, 1, 0)
    );

    let posts = out.path().join("r_imaging/media/posts");
    assert!(!posts.join("big1.png").exists());
    let big = image::open(posts.join("big1.jpg")).unwrap();
    assert_eq!((big.width(), big.height()), (1344, 672));
    assert_eq!(
        std::fs::read(posts.join("small1.jpg")).unwrap(),
        jpeg(300, 200),
        "matching images are stored byte-for-byte"
    );
}

#[tokio::test]
async fn conversion_never_touches_gifs() {
    let server = MockServer::start().await;
    mount_gifs(&server, false).await;
    let out = tempdir().unwrap();

    let mut config = config(&server, out.path(), Target::subreddit("gifs").unwrap());
    config.media.formats = vec![MediaFormat::Gif];
    config.transforms.convert = Some(ConvertFormat::Jpg);
    config.transforms.max_size = Size::new(10, 10);
    let media = run(&config).await.media;
    assert_eq!(
        (media.downloaded, media.converted, media.resized),
        (1, 0, 0)
    );
    assert_eq!(
        std::fs::read(out.path().join("r_gifs/media/posts/a1.gif")).unwrap(),
        b"FAKE-/i/animated.gif"
    );
}
