# Development

## Build and check

The crate requires Rust 1.99+ (edition 2024); `rust-toolchain.toml`, CI and
Docker pin Rust 1.99.1. Building the TLS dependency requires CMake, Perl, a
C/C++ compiler, and libclang.

```sh
cargo build --release --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --lib --no-default-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features --locked
```

The `cli` feature (default) builds the `reddit` binary; library users can
disable default features to avoid clap.

Tests use generated fixtures and a mock HTTP server; no Reddit credentials or
live network access are required. A small authenticated smoke test is separate:

```sh
cargo run --release --locked -- pics --posts 5 --offline --cookies cookies.txt --out-dir output/smoke
# Repeat to check caching and merging, then open output/smoke/index.html.
```

Do not commit cookies, generated archives, or logs. Optional local commit checks:

```sh
git config core.hooksPath .githooks
```

The npm proxy and release helpers use Node.js 22+ and have no build-time npm
dependencies:

```sh
npm test --prefix npm
node scripts/release.mjs verify
npx --yes prettier@3.9.10 --check npm scripts .github/workflows README.md docs
actionlint
```

## Source layout

| Path                   | Responsibility                                                       |
| ---------------------- | -------------------------------------------------------------------- |
| `src/lib.rs`           | Crate documentation and public API re-exports                        |
| `src/main.rs`          | CLI arguments (`cli` feature), per-target runs, summary              |
| `src/pipeline.rs`      | `Config`, run orchestration                                          |
| `src/target.rs`        | Validated targets, name/path/URL parsing, sort/time hints            |
| `src/cookies.rs`       | Netscape and inline cookie parsing                                   |
| `src/http.rs`          | Shared browser-emulating HTTP client, retry and rate-limit policy    |
| `src/client.rs`        | Listing/about API requests and pagination                            |
| `src/normalize.rs`     | Raw Reddit JSON to models                                            |
| `src/models.rs`        | Domain types and serialization                                       |
| `src/manifest.rs`      | Media selection: formats, sizes, gallery limits, planned conversions |
| `src/download.rs`      | Cached, parallel media downloads with fallbacks and format checks    |
| `src/transform.rs`     | Still-image conversion and downscaling                               |
| `src/archive.rs`       | Archive files, atomic writes, incremental merge, archive discovery   |
| `src/html.rs`          | Viewer rendering; templates live in `src/viewer/`                    |
| `tests/integration.rs` | Mocked end-to-end tests                                              |
| `examples/archive.rs`  | Runnable library example                                             |
| `npm/`                 | Thin native CLI proxy and platform package mapping                   |
| `scripts/release.mjs`  | Version checks, archives, npm staging, checksums, formula generation |
| `Formula/reddit.rb`    | Homebrew tap formula, updated automatically after releases           |
| `.github/workflows/`   | CI, crate publishing, multi-platform Docker publishing               |

Comments should explain constraints or non-obvious behavior, not repeat the
code. Record user-visible changes in `CHANGELOG.md`.

## Releases

1. Update `Cargo.toml`, regenerate `Cargo.lock`, and synchronize `npm/package.json`
   (including every optional dependency) and version examples in `docs/`.
2. Run the checks above. Inspect `cargo package --list --locked` for secrets or
   generated output. Use `cargo publish --dry-run --locked` to verify the crate.
3. Commit, create an annotated stable `v<version>` tag, and push the branch and tag.

`release.yml` verifies versions, tests and builds six native targets, and smoke-tests
the npm proxy on every platform. It assembles checksum-verified assets in a draft
GitHub release, publishes all native npm packages before the launcher, then publishes
the release and updates the Homebrew formula on `main`. Crate and Docker publishing
run afterward through reusable workflows. Publishing a release with `GITHUB_TOKEN`
does not trigger other workflows, so these calls are explicit.

Native Windows ARM64 builds set `CMAKE_TOOLCHAIN_FILE` to the checked-in
`.github/cmake/windows-arm64.cmake` workaround for btls-sys issue #151. It disables
assembly that MSBuild cannot compile and matches Rust's static CRT.

Required repository secrets: `NPM_TOKEN` (publish rights to the `@rddt` scope) and
`CARGO_REGISTRY_TOKEN` (the `crates-io` environment may supply it). npm publishing
includes provenance. The workflow's `GITHUB_TOKEN` must be allowed to push to `main`
for formula updates; branch protection must permit the bot or the push will fail.

The separate `reddit-rs/homebrew-tap` repository updates itself from published
release assets, periodically or through its **Update reddit** workflow. It uses
its own repository-scoped `GITHUB_TOKEN`; no personal access token, deploy key,
or cross-repository secret is required. Formula updates never downgrade versions.

Docker images build on native x64/ARM runners, with architecture-specific locked
Cargo caches, before their digests are combined into a multi-platform manifest.
The public npm smoke test uses fresh anonymous caches and bounded retries to
handle registry propagation. Authorization failures are not retried.

Retry a failed release with **Release binaries and installers → Run workflow**,
specifying its existing tag. npm retries verify published package integrity rather
than overwriting versions. Published GitHub assets are also checked, not overwritten.
Do not move release tags. The Homebrew update creates a separate bot commit; pull
`main` after the release completes.
