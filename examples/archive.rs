//! Archive a whole listing with the library API and write the offline viewer.
//!
//! ```sh
//! cargo run --release --example archive -- rust
//! cargo run --release --example archive -- 'https://www.reddit.com/r/pics/top/?t=week' cookies.txt
//! ```

use reddit::{Config, Cookies, PostsMode, Sort};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let parsed = reddit::parse_target(&args.next().unwrap_or_else(|| "rust".into()))?;

    let mut config = Config::new(parsed.target);
    config.sort = parsed.sort.unwrap_or(Sort::New);
    config.time = parsed.time.unwrap_or_default();
    config.posts = PostsMode::All;
    config.offline = true;
    if let Some(cookies) = args.next() {
        // A Netscape cookies.txt path or an inline 'name=value; …' string.
        config.cookies = Cookies::from_arg(&cookies)?;
    }

    let summary = reddit::run(&config).await?;
    let media = summary.media;
    println!(
        "{} posts ({} new) | media: {} downloaded, {} cached, {} failed",
        summary.posts, summary.posts_new, media.downloaded, media.cached, media.failed
    );
    Ok(())
}
