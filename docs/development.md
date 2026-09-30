# Development

## Build and check

The package declares Rust 1.88+ (edition 2024). CI and Docker pin Rust 1.98.1;
use that toolchain for release parity. Building the TLS dependency requires
CMake, Perl, a C/C++ compiler, and libclang.

```sh
cargo build --release --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features --locked
```

Tests use generated fixtures and a mock HTTP server; no Reddit credentials or
live network access are required. A small authenticated smoke test is separate:

```sh
cargo run --locked -- pics --posts 5 --offline --cookies cookies.txt --out-dir output/smoke
# Repeat to check caching, then open output/smoke/index.html.
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
npx --yes prettier@3.9.9 --check npm scripts .github/workflows README.md docs
actionlint
```

## Source layout

| Path                   | Responsibility                                                       |
| ---------------------- | -------------------------------------------------------------------- |
| `src/lib.rs`           | Public modules and API re-exports                                    |
| `src/main.rs`          | CLI argument parsing, target runs, summary                           |
| `src/target.rs`        | Target validation and URL hints                                      |
| `src/pipeline.rs`      | Configuration, incremental merge, persistence, orchestration         |
| `src/storage.rs`       | Shared atomic archive writes                                         |
| `src/client.rs`        | Listing HTTP requests, pagination, retries                           |
| `src/clean.rs`         | API normalization, manifest construction, parsing helpers            |
| `src/models.rs`        | Shared data types and serialization                                  |
| `src/download.rs`      | Media paths, caching, parallel downloads, atomic writes              |
| `src/transform.rs`     | Still-image conversion and downscaling                               |
| `src/html.rs`          | Self-contained viewer and archive hub                                |
| `tests/integration.rs` | Mocked end-to-end tests                                              |
| `examples/archive.rs`  | Runnable library example                                             |
| `npm/`                 | Thin native CLI proxy and platform package mapping                   |
| `scripts/release.mjs`  | Version checks, archives, npm staging, checksums, formula generation |
| `Formula/reddit.rb`    | Homebrew tap formula, updated automatically after releases           |
| `.github/workflows/`   | CI, crate publishing, multi-platform Docker publishing               |

Keep archive formats and public API compatible during maintenance refactors.
Comments should explain constraints or non-obvious behavior, not repeat the code.

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
