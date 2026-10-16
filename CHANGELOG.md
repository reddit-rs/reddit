# Changelog

## 0.6.0 - 2026-10-16

This release restructures the crate. Archives and the library API are not
compatible with 0.5; re-run existing archives into a fresh output directory.

### Breaking

- Archive files are named `posts.json`, `about.json`, `raw.json` and
  `manifest.json` (previously `<name>_posts.json`, `<name>_about.json`,
  `<name>_posts_raw.json` and `media_manifest.json`).
- Media extensions are canonical: `.jpeg` sources are stored as `.jpg`.
- Manifest entries use `post_id`, a typed `kind` and `format` instead of
  `id`, `folder` and `ext`. Gallery items report `animated` instead of `gif`.
- Library: `Config::new(target)` replaces `Config::default()`; options are
  grouped into `Selection` and `TransformOptions`; `run` takes `&Config`;
  `Target` is validated on construction; `Cookies` replaces raw cookie
  strings; `PostsMode` is `FirstPage`, `Limit(n)` or `All`; `Summary.media`
  replaces the flat `media_*` counters. Internal modules are private.
- `--user-agent` defaults to the emulated browser's own User-Agent.
- Rust 1.99 or newer is required.

### Fixed

- The TLS fingerprint (Chrome 131) did not match the default User-Agent
  (Chrome 151); requests now consistently emulate Chrome 149.
- Gallery items with a video MIME type were saved as `.mp4` "stills"; they now
  use their preview frame.
- A nonexistent `--cookies` path was silently treated as an inline cookie;
  unreadable files are errors and expired cookies are ignored. One invalid
  cookie no longer drops the whole `Cookie` header.
- A corrupt `posts.json` was replaced by a new archive; the run now stops.
- Image conversion no longer blocks the async runtime.
- Permanent media errors (4xx) are no longer retried; `Retry-After` and
  Reddit's rate-limit headers are honored.
- Downloads are checked by content sniffing, so a converted file can no longer
  contain GIF data under a `.jpg` name.
- The viewer only links to `http(s)` URLs and sets the banner through the CSSOM.

### Changed

- The `clap` dependency is behind the default `cli` feature.
- Unused `regex` dependency and dead helpers were removed.
- Dependencies, Rust (1.99.1), Node.js (24.22.0) and GitHub Actions were updated.
