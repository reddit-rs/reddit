# reddit

Archive Reddit listings as JSON and media, with an optional offline HTML viewer.
Supports subreddits, users, galleries, multiple targets, and incremental updates.

![An archived r/pics post open in the offline viewer's image lightbox](https://raw.githubusercontent.com/reddit-rs/reddit/main/screenshot.png)

## Quick start

```sh
npx @rddt/cli pics --cookies cookies.txt --offline
# Or use an installed native binary:
reddit pics --cookies cookies.txt --offline
```

## Installation

**Homebrew** (macOS/Linux):

```sh
brew tap reddit-rs/reddit https://github.com/reddit-rs/reddit
brew install reddit-rs/reddit/reddit
```

**npm** (macOS/Linux/Windows; Node.js 22+):

```sh
npm install -g @rddt/cli
rddt --help
```

Or use `npx @rddt/cli` without a global installation. JavaScript only launches
the matching native binary; Rust is not required.

[Release binaries](https://github.com/reddit-rs/reddit/releases/latest) support
x64 and arm64 on all three operating systems. See [distribution details](docs/install.md)
for requirements and checksums. To build from source: `cargo install reddit --locked`.

Export a Netscape-format `cookies.txt` from a browser signed in to Reddit.
Treat it as a password: never share or commit it. Anonymous requests may return
HTTP 403; cookies only grant access to content your account can already view.

Open `output/index.html` to browse your archives. By default, each run fetches
the first listing page (about 25 posts), downloads images, and skips videos.
Re-runs merge posts and reuse nonempty media files already on disk.

```sh
# Archive several listings, up to the API's pagination limit.
reddit pics rust --posts 0 --offline --cookies cookies.txt

# Keep large still images, downscale, and convert to JPEG.
reddit pics --min-size 768x1024 --max-size 1344x1792 --convert jpg --cookies cookies.txt

# Select a format or use listing hints from a URL.
reddit gifs --formats gif --cookies cookies.txt
reddit 'https://www.reddit.com/r/rust/top/?t=week' --cookies cookies.txt
```

Downloaded media works offline. Missing media may fall back to remote URLs and
requires a connection. Reddit-hosted videos are saved without their separate
audio track. Listings are API-limited; `--posts 0` is not a complete history.

## Documentation

- [Usage](docs/usage.md): options, authentication, output, and caching
- [Docker](docs/docker.md): published images and local builds
- [Installation](docs/install.md): binaries, Homebrew, and npm
- [Development](docs/development.md): source layout, checks, and releases
- [Library API](https://docs.rs/reddit) · [Example](examples/archive.rs)

Run `reddit --help` for the full CLI reference.

## License

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE), at your option.
Contributions are dual-licensed under the same terms unless explicitly stated otherwise.

This project is not affiliated with Reddit, Inc. Content belongs to its respective
owners. Archive only content you are allowed to access and respect Reddit's
[User Agreement](https://www.redditinc.com/policies/user-agreement) and rate limits.
