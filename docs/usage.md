# Usage

## Targets and listing options

```sh
reddit pics rust u/spez --posts 50 --offline --cookies cookies.txt
reddit 'https://www.reddit.com/r/rust/top/?t=week' --cookies cookies.txt
reddit r/rust+golang --sort new --cookies cookies.txt
```

Each target gets its own `r_<name>` or `u_<name>` directory. A `+` target combines
subreddits into one archive. Explicit sort/time flags override URL hints.

| Option           | Behavior                                                                                         |
| ---------------- | ------------------------------------------------------------------------------------------------ |
| `--posts [N]`    | Paginate up to N posts; `0` or no value: until the listing ends. Omitted: first page (25 posts). |
| `--sort ORDER`   | `hot` (default), `new`, `top`, `rising`, `controversial`                                         |
| `--time WINDOW`  | `hour`, `day`, `week`, `month`, `year`, `all` (default); for top/controversial                   |
| `--since DATE`   | Keep fetched posts created on/after `YYYY-MM-DD` (UTC); archived posts stay.                     |
| `--out-dir DIR`  | Output root; default `output`, or `REDDIT_OUT_DIR`. Explicit flag wins.                          |
| `--offline`      | Generate a viewer per archive and a root archive hub.                                            |
| `--no-raw`       | Do not save the raw listing (`raw.json`).                                                        |
| `--no-downloads` | Save JSON without downloading media.                                                             |

Reddit limits listing history. Pagination stops when the listing ends, stops
adding posts, or reaches the 200-page safety limit.

## Media options

| Option               | Behavior                                                                                                                |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `--videos`           | Include Reddit-hosted videos (silent; audio is a separate track).                                                       |
| `--formats LIST`     | Comma-separated or repeated: `jpg`, `jpeg`, `png`, `gif`, `webp`, `bmp`, `mp4`, `webm`. Video formats imply `--videos`. |
| `--gallery-images N` | Maximum items per gallery; `0` (default) keeps all.                                                                     |
| `--no-icon`          | Skip subreddit icon/banner downloads.                                                                                   |
| `--min-size WxH`     | Require still-image short side ≥ W and long side ≥ H, regardless of orientation.                                        |
| `--max-size WxH`     | Fit stills within a width/height box; preserve aspect ratio, never upscale.                                             |
| `--convert FORMAT`   | Convert JPEG/PNG/BMP stills to `jpg` (or `jpeg`) or `png`.                                                              |
| `--quality N`        | JPEG quality for rewritten images, 1–100; default 85.                                                                   |

Format selection uses URL parameters and MIME types; `jpeg` and `jpg` are
equivalent. Known response-format mismatches are rejected. Unknown formats are
excluded when a format selection is set. Filtering does not remove post metadata
or delete existing files.

The size floor uses Reddit's reported dimensions before downloading; unknown
dimensions are kept. Subreddit art and videos are exempt. GIF/WebP files are not
converted or resized because they may be animated.

Transforms apply only to new downloads. Images already matching the format and
size remain byte-for-byte unchanged. JPEG conversion flattens transparency onto
white. Rewritten originals are not retained; use a separate output directory if
you need both original and transformed archives.

## Authentication

```sh
reddit pics --cookies cookies.txt
reddit pics --cookies 'reddit_session=…; token_v2=…'
```

A path to an existing file is read as a Netscape cookie export; anything else must
be an inline `name=value; …` string, so a mistyped path is reported instead of
being sent as a cookie. From exports, only unexpired cookies for `reddit.com` and
its subdomains are used, including `#HttpOnly_` entries. File-based cookies are
preferable to inline values, which may enter shell history and process lists.
Keep exports private and out of version control (`cookies.txt` is ignored).
Session cookies are only sent to HTTPS Reddit origins, never to media CDNs or
linked sites.

Requests use a Chrome 149 TLS/HTTP2 fingerprint with its matching User-Agent.
If your cookies come from another browser, pass its User-Agent with
`--user-agent`. The client adds `over18=1` unless supplied, pauses between
listing pages, honors `Retry-After` and Reddit's rate-limit headers, and retries
transient failures with backoff. Private, quarantined, or age-restricted listings
still require an account with access. Authentication does not guarantee Reddit
will accept every request.

## Archive layout

```text
output/
  index.html                  # archive hub (--offline)
  r_pics/
    posts.json                # merged posts and listing metadata
    about.json                # subreddit metadata (single subreddits only)
    raw.json                  # latest fetched listing (--no-raw to skip)
    manifest.json             # selected media (--no-downloads to skip)
    index.html                # viewer (--offline)
    media/
      subreddit_icon.png
      subreddit_banner.jpg
      posts/
        <id>.jpg              # image post
        <id>_00.jpg           # gallery items in display order
        <id>_01.png
        <id>.mp4              # video (--videos)
```

Extensions are canonical (`.jpg`, never `.jpeg`) and follow the format Reddit
actually serves, including resizer `format=` parameters and `--convert`.

## Incremental runs

Fresh posts retain listing order and updated metadata; older posts remain in the
archive, newest first. Raw JSON describes only the latest fetch, not the merge.
`--since` filters incoming posts, not the existing archive.

Each run reads `posts.json` first; if it cannot be parsed the run stops without
overwriting it. A nonempty regular media file is cached without another request. Downloads use
a temporary `.part` sibling and rename only after completion; JSON and HTML writes
use the same replacement strategy. Cached files are
not rewritten when transform settings change. Downloads are checked against
their expected format; content of another format is rejected and the fallback
URL is tried. Permanent HTTP errors are not retried.
Avoid concurrent writers to the same archive directory.

The viewer uses local media first, then remote fallbacks. A filtered gallery item
may be hidden; missing remote files will not work without a connection.
