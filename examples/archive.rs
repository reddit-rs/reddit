//! Minimal programmatic usage of the `reddit` crate.
//!
//! This mirrors the CLI: build a [`Config`](reddit::Config), call
//! [`reddit::run`], and get a [`Summary`](reddit::Summary) back.
//!
//! Run it:
//!
//! ```sh
//! cargo run --release --example archive -- rust
//! cargo run --release --example archive -- funny /path/to/cookies.txt
//! ```

use reddit::{Config, PostsMode, Sort};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let input = args.first().map(String::as_str).unwrap_or("rust");
    let parsed = reddit::parse_target(input)?;

    let mut cfg = Config {
        target: parsed.target,
        sort: parsed.sort.unwrap_or(Sort::New),
        posts: PostsMode::All(0), // paginate the whole listing
        offline: true,            // also write index.html
        ..Config::default()
    };
    if let Some(path) = args.get(1) {
        cfg.cookies = Some(path.clone()); // Netscape cookies.txt or 'k=v; k2=v2'
    }

    let summary = reddit::run(cfg).await?;
    println!(
        "{} posts | media: {} downloaded, {} failed",
        summary.posts, summary.media_total, summary.media_failed
    );
    Ok(())
}
