# reddit

> **Disclaimer:** This project is **not affiliated with, endorsed by, or connected to
> Reddit, Inc.** It is an unofficial, independent offline viewer built on Reddit's
> public JSON endpoints. All content belongs to its respective owners — use it only to
> view and archive content you are allowed to access, and respect Reddit's
> [User Agreement](https://www.redditinc.com/policies/user-agreement) and rate limits.

An offline viewer for reddit — archive a subreddit for local use: structured JSON,
every image (galleries included), and a self-contained HTML index you can browse
without a network connection. Content is downloaded once and lives on your disk.

## Quick start

Install from crates.io, then run:

```sh
cargo install reddit
reddit funny --cookies cookies.txt
```

That saves the first page of the listing (~25 posts) and their images into
`output/r_funny/`. Galleries keep every image; videos are skipped by
default. The folder is your archive — JSON data, media files and (with `--offline`)
a single `index.html` that opens from disk in any browser.

> Reddit rejects anonymous JSON requests from many networks with `HTTP 403`, so a
> `cookies.txt` exported from your browser (see [Authentication](#authentication))
> is the reliable way to fetch any listing. `--cookies` is only optional where
> reddit still allows anonymous access.

Want more?

```sh
# the 5 newest posts with their galleries, as JSON + images
reddit funny --sort new --posts 5 --cookies cookies.txt

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
- the first page of posts (`<name>_posts.json`, ~25 posts, full metadata)
- the images of those posts (`media/posts/…`)

Galleries are downloaded in full (every image, in order), and each gallery image
prefers the original `i.redd.it` file with the signed preview URL as a fallback.
Videos, pagination depth and subreddit art are opt-in / configurable.

## Options

```
--posts [N]          paginate the listing; optional cap (0 = everything; default: first page)
--sort ORDER         hot, new, top, rising or controversial (default: hot)
--time WINDOW        hour, day, week, month, year or all (for top/controversial)
--videos             also download reddit-hosted video files (images only by
                      default; reddit serves audio as a separate track, so these
                      files are silent)
--gallery-images N   max images per gallery (0 = all)
--since DATE         only posts created on/after YYYY-MM-DD
--cookies STR|FILE   'k=v; k2=v2' or a Netscape cookies.txt path (recommended;
                      reddit rejects anonymous JSON requests; unlocks private/NSFW)
--user-agent STRING  must match the browser the cookies were exported from
--out-dir DIR        output directory (default: output/<r_|u_><name>)
--no-icon            skip the subreddit icon and banner
--no-raw             skip the raw listing JSON dump
--no-downloads       JSON only, no media
--offline            generate a self-contained index.html to browse the archive
```

Targets can be names (`funny`), reddit paths (`r/rust`, `u/spez`) or
full URLs (`https://www.reddit.com/r/rust/top/?t=week`), including multi-subreddits
(`r/rust+golang`). Sort/time hints embedded in a URL are used when the flags are
not given.

## Output layout

```
output/r_<name>/
  <name>_about.json        subreddit metadata
  <name>_posts.json        clean posts (galleries, previews, stats)
  <name>_posts_raw.json    raw listing JSON (--no-raw to skip)
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

## Offline HTML viewer

Add `--offline` and the run also produces a single, self-contained `index.html`
next to the data — open it in any browser (even from disk, no server needed) and
browse the archive: subreddit header, a card per post, and a lightbox for images,
galleries, self posts and videos with scores and metadata.

Downloaded files are served from `media/`; anything missing falls back to the
original reddit CDN URL, so the page also works for `--no-downloads` runs.

```sh
reddit funny --posts 0 --offline --cookies cookies.txt
# then open output/r_funny/index.html
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

```sh
docker build -t reddit:local .
docker run --rm \
  -v "$PWD/output:/data" \
  -v "$PWD/cookies.txt:/cookies.txt:ro" \
  reddit:local funny --cookies /cookies.txt
```

The container runs as a non-root user (uid 10001); if you hit a permission error
on the mount, make the host directory writable: `chmod -R a+w output`.

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

MIT
