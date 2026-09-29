# reddit

<img src="https://raw.githubusercontent.com/reddit-rs/reddit/refs/heads/main/screenshot.png" alt="The reddit offline viewer browsing an archived r/funny listing — subreddit header with icon and member count, and a grid of saved post images with titles.">

> **Disclaimer:** This project is **not affiliated with, endorsed by, or connected to
> Reddit, Inc.** It is an unofficial, independent offline viewer built on Reddit's
> public JSON endpoints. All content belongs to its respective owners — use it only to
> view and archive content you are allowed to access, and respect Reddit's
> [User Agreement](https://www.redditinc.com/policies/user-agreement) and rate limits.

An offline viewer for reddit — archive one or more subreddits for local use:
structured JSON, every image (galleries included), and a self-contained HTML
index you can browse without a network connection. Content is downloaded once
and lives on your disk; re-runs only fetch what is new.

## Quick start

Install from crates.io, then run:

```sh
cargo install reddit
reddit funny --cookies cookies.txt
```

That saves the first page of the listing (~25 posts) and their images into
`output/r_funny/`. Galleries keep every image; videos are skipped by
default. The folder is your archive — JSON data, media files and (with `--offline`)
an `index.html` that opens from disk in any browser.

> Reddit rejects anonymous JSON requests from many networks with `HTTP 403`, so a
> `cookies.txt` exported from your browser (see [Authentication](#authentication))
> is the reliable way to fetch any listing. `--cookies` is only optional where
> reddit still allows anonymous access.

Want more?

```sh
# the 5 newest posts with their galleries, as JSON + images
reddit funny --sort new --posts 5 --cookies cookies.txt

# only animated GIFs (other formats are not downloaded)
reddit gifs --formats gif --posts 0 --cookies cookies.txt

# several subreddits in one run, each in its own output directory
reddit funny rust golang --posts 0 --offline --cookies cookies.txt

# paginate the whole listing, sort by top of the year, download videos too
reddit funny --posts 0 --sort top --time year --videos --cookies cookies.txt

# private / NSFW subreddits: the same cookies file
reddit SomePrivateSub --cookies cookies.txt

# complete offline archive: everything + a browsable index.html
reddit rust --posts 0 --offline --cookies cookies.txt
```

## What it does

By default it saves what the listing's first page shows:

- listing metadata (`<name>_about.json`) — title, description, subscribers, icon/banner
- the first page of posts (`<name>_posts.json`, ~25 posts, full metadata; re-runs
  merge into it instead of replacing it)
- the images of those posts (`media/posts/…`)

Galleries are downloaded in full (every image, in order), and each gallery image
prefers the original `i.redd.it` file with the signed preview URL as a fallback.
Videos, pagination depth and subreddit art are opt-in / configurable. Re-runs
merge the fresh listing into the existing archive and never download media that
is already on disk — see [Re-runs and caching](#re-runs-and-caching).

## Options

```
--posts [N]          paginate the listing; optional cap (0 = everything; default: first page)
--sort ORDER         hot, new, top, rising or controversial (default: hot)
--time WINDOW        hour, day, week, month, year or all (for top/controversial)
--videos             also download reddit-hosted video files (images only by
                      default; reddit serves audio as a separate track, so these
                      files are silent)
--formats LIST       only download these formats, e.g. 'gif' or 'jpg,jpeg,png'
                      (repeatable/comma-separated; mp4/webm imply --videos)
--gallery-images N   max images per gallery (0 = all)
--since DATE         only posts created on/after YYYY-MM-DD
--cookies STR|FILE   'k=v; k2=v2' or a Netscape cookies.txt path (recommended;
                      reddit rejects anonymous JSON requests; unlocks private/NSFW)
--user-agent STRING  must match the browser the cookies were exported from
--out-dir DIR        output directory (default: output/<r_|u_><name>; $REDDIT_OUT_DIR)
--no-icon            skip the subreddit icon and banner
--no-raw             skip the raw listing JSON dump
--no-downloads       JSON only, no media
--offline            per-archive index.html viewer + a root archive hub
```

Targets can be names (`funny`), reddit paths (`r/rust`, `u/spez`) or
full URLs (`https://www.reddit.com/r/rust/top/?t=week`), including multi-subreddits
(`r/rust+golang`). Sort/time hints embedded in a URL are used when the flags are
not given. Pass several targets to archive them in one run — each goes to its own
directory under `--out-dir`.

### Format filtering

`--formats` keeps only the media containers you ask for:

```sh
reddit gifs --formats gif --posts 0 --cookies cookies.txt
reddit pics --formats jpg,jpeg,png --cookies cookies.txt
reddit video --formats mp4 --videos --cookies cookies.txt
```

The decision is made from reddit's own mime types and each URL's `format=`
parameter, so `jpeg` and `jpg` are one format and a `….png?format=pjpg` preview
is treated as the JPEG it really is — and stored with a `.jpg` name. The
downloader also verifies the response `content-type`, so a fallback of the wrong
format is never saved under a mismatched name. When a selection is set, media
whose format cannot be determined is skipped rather than guessed at.

The listing JSON still describes every post; filtered-out files are simply not
downloaded, the offline viewer falls back to the original reddit URL for them
(or hides the gallery item), and files already in `media/` are never deleted.

## Output layout

```
output/
  index.html                 archive hub linking every r_*/u_* directory (--offline)
  r_<name>/
    <name>_about.json        subreddit metadata
    <name>_posts.json        clean posts (galleries, previews, stats)
    <name>_posts_raw.json    raw listing JSON of the latest run (--no-raw to skip)
    media_manifest.json      every downloadable file
    media/
      subreddit_icon.png
      subreddit_banner.jpg
      posts/
        <post id>.jpg        single image post
        <post id>_00.jpg     gallery images, in display order
        <post id>_01.png
        <post id>_02.mp4     gallery video / animated item (--videos)
        <post id>.mp4        reddit-hosted video (--videos)
    index.html               offline viewer (--offline)
```

JSON always contains the full data from the listing (all gallery children, captions,
scores, flairs, …). Download depth is controlled by the flags above.

## Re-runs and caching

Archives are incremental. The media downloader checks the file system first —
anything already in `media/` is reported as cached and never requested again, so
re-running an archive only fetches what is new:

```sh
reddit funny --posts 0 --offline --cookies cookies.txt   # first archive
reddit funny --posts 0 --offline --cookies cookies.txt   # media: 0 downloaded, 42 cached
```

Downloads are written to a `.part` file and renamed into place, so an interrupted
run never leaves a truncated file that a later run would mistake for finished
content.

The fresh listing is also merged into `<name>_posts.json`: posts reddit still
returns keep their updated scores, while previously archived posts that have
since dropped off the listing stay in the archive (newest first). `--since`
filters what is fetched, it does not delete stored posts.

## Offline HTML viewer

Add `--offline` and the run also produces a single, self-contained `index.html`
next to the data — open it in any browser (even from disk, no server needed) and
browse the archive: subreddit header, a card per post, and a lightbox for images,
galleries, self posts and videos with scores and metadata.

Downloaded files are served from `media/`; anything missing falls back to the
original reddit CDN URL, so the page also works for `--no-downloads` runs.

With `--offline`, the output root also gets an `index.html` hub that links every
archive in the folder (with icon, post count and fetch date); each archive page
links back to it. Archiving a second subreddit later updates the hub, so several
archives are one click apart:

```sh
reddit funny rust --posts 0 --offline --cookies cookies.txt
# then open output/index.html for both archives, or output/r_funny/index.html
```

## Authentication

Cookies are the reliable way to fetch any listing (reddit rejects anonymous JSON
requests from many networks) and they unlock private, quarantined and NSFW
listings. Pass them either as a string or as a Netscape-format file exported
from your browser:

```sh
reddit SomeSub --cookies "reddit_session=…; token_v2=…; csrf_token=…"
reddit SomeSub --cookies cookies.txt
```

`#HttpOnly_` cookie lines (which browsers use for `reddit_session` and
`token_v2`) are read like any other cookie. The `--user-agent` should match the
browser the cookies were exported from; an `over18=1` opt-in cookie is added
automatically. The cookies file is never committed — it is in `.gitignore`.

## Anti-bot notes

Reddit rejects plain HTTP clients from many networks with `403`/`429`. This tool
uses [`wreq`](https://crates.io/crates/wreq) with a Chrome TLS emulation profile,
respects rate limits (page delay), and backs off on `429`. A signed-in cookie
file is still the most reliable way to fetch NSFW/private content.

## Install & build

Requires Rust 1.97+ (edition 2024), plus cmake/perl to build BoringSSL (used by
the TLS emulation).

```sh
cargo build --release
cargo install --path .
```

## Docker

Published images are on GitHub Container Registry (`linux/amd64` and
`linux/arm64`, built on `alpine:3.24` with the musl binary — about 8 MB
compressed, ~17 MB on disk):

```sh
docker run --rm \
  -v "$PWD/output:/data" \
  -v "$PWD/cookies.txt:/cookies.txt:ro" \
  ghcr.io/reddit-rs/reddit funny --cookies /cookies.txt
```

The `latest` tag tracks the newest release; pin a version for reproducible
archives (`ghcr.io/reddit-rs/reddit:0.3.0`). Multiple subreddits, format
filtering and the offline viewer work the same way:

```sh
docker run --rm \
  -v "$PWD/output:/data" \
  -v "$PWD/cookies.txt:/cookies.txt:ro" \
  ghcr.io/reddit-rs/reddit funny rust --posts 0 --offline --cookies /cookies.txt

docker run --rm \
  -v "$PWD/output:/data" \
  -v "$PWD/cookies.txt:/cookies.txt:ro" \
  ghcr.io/reddit-rs/reddit gifs --formats gif --cookies /cookies.txt
```

Archives land in `./output/` (the image sets `REDDIT_OUT_DIR=/data`, and the same
variable overrides `--out-dir` anywhere). The container runs as a non-root user
(uid 10001); if you hit a permission error on the mount, make the host directory
writable: `chmod -R a+w output`.

To build the image from source instead:

```sh
docker build -t reddit:local .
docker run --rm \
  -v "$PWD/output:/data" \
  -v "$PWD/cookies.txt:/cookies.txt:ro" \
  reddit:local funny --cookies /cookies.txt
```

## Library usage

The same pipeline is available programmatically:

```rust
use reddit::{Config, PostsMode, Sort};

let cfg = Config {
    target: reddit::parse_target("funny")?.target,
    posts: PostsMode::All(0), // paginate everything
    sort: Sort::New,
    offline: true,            // also write index.html
    ..Config::default()
};
let summary = reddit::run(cfg).await?;
println!("{} posts, {} media files", summary.posts, summary.media_total);
```

## Tests

```sh
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check

# optional: run the same checks on every commit
git config core.hooksPath .githooks
```

Unit tests cover target/cookie parsing, post and gallery cleaning and manifest
building; integration tests run the whole pipeline against a mocked reddit API
(wiremock) — no network access required.

## Project layout

```
src/
  main.rs       CLI (clap)
  lib.rs        Config, run() pipeline, target parsing
  client.rs     HTTP client (wreq + Chrome TLS emulation), listing pagination
  clean.rs      raw API JSON -> clean models, manifest builder, cookie parsing
  models.rs     data structures
  download.rs   parallel media downloader (caching, retries, URL fallbacks)
  html.rs       self-contained offline viewer
tests/
  integration.rs  end-to-end tests against a mocked API
.github/workflows/
  ci.yml          fmt / check / clippy / tests (ubuntu + macOS)
  publish-crates.yml
  publish-docker.yml
```

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
