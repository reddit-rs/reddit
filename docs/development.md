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

## Source layout

| Path | Responsibility |
| --- | --- |
| `src/lib.rs` | Public modules and API re-exports |
| `src/main.rs` | CLI argument parsing, target runs, summary |
| `src/target.rs` | Target validation and URL hints |
| `src/pipeline.rs` | Configuration, incremental merge, persistence, orchestration |
| `src/storage.rs` | Shared atomic archive writes |
| `src/client.rs` | Listing HTTP requests, pagination, retries |
| `src/clean.rs` | API normalization, manifest construction, parsing helpers |
| `src/models.rs` | Shared data types and serialization |
| `src/download.rs` | Media paths, caching, parallel downloads, atomic writes |
| `src/transform.rs` | Still-image conversion and downscaling |
| `src/html.rs` | Self-contained viewer and archive hub |
| `tests/integration.rs` | Mocked end-to-end tests |
| `examples/archive.rs` | Runnable library example |
| `.github/workflows/` | CI, crate publishing, multi-platform Docker publishing |

Keep archive formats and public API compatible during maintenance refactors.
Comments should explain constraints or non-obvious behavior, not repeat the code.

## Releases

1. Update `Cargo.toml`, regenerate `Cargo.lock`, and update the Docker version in
   `docs/docker.md`.
2. Run the checks above and `cargo publish --dry-run --locked`. Inspect
   `cargo package --list --locked` for secrets or generated output.
3. Commit the verified changes, create an annotated `v<version>` tag, and push
   the branch and tag.
4. Publish a GitHub release for the tag to trigger crate and container publishing.
   A tag push alone does not publish either artifact.

Crate publishing requires the `crates-io` environment and registry token.
The release workflow verifies that the tag matches the package version.
