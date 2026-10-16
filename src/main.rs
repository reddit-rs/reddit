use anyhow::{Result, bail};
use chrono::NaiveDate;
use clap::Parser;
use reddit::{
    Config, ConvertFormat, Cookies, MediaFormat, MediaReport, PostsMode, Selection, Size, Sort,
    Summary, TimeFilter, TransformOptions,
};
use std::num::NonZeroU64;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "reddit",
    version,
    about = "Archive Reddit listings as JSON and media, with an offline viewer",
    long_about = "Archive subreddit or user listings as JSON and media. By default a run \
fetches the first listing page (~25 posts) and downloads images but not videos. \
Re-runs merge posts and reuse media already on disk. Reddit usually requires cookies \
exported from a signed-in browser.",
    after_help = "Examples:\n  \
reddit pics --offline --cookies cookies.txt\n  \
reddit pics rust --posts 0 --offline --cookies cookies.txt\n  \
reddit gifs --formats gif --cookies cookies.txt"
)]
struct Cli {
    /// Subreddits or users: name, r/name, u/name or a Reddit URL
    #[arg(value_name = "SUBREDDIT_OR_URL", required = true)]
    targets: Vec<String>,

    /// Netscape cookies.txt file, or an inline 'name=value; name2=value2' string
    #[arg(long, value_name = "FILE|COOKIES")]
    cookies: Option<String>,

    /// User-Agent of the browser the cookies came from [default: Chrome 149]
    #[arg(long)]
    user_agent: Option<String>,

    /// Output root; each target is saved to <DIR>/<r_|u_><name>
    #[arg(
        long,
        default_value = "output",
        env = "REDDIT_OUT_DIR",
        value_name = "DIR"
    )]
    out_dir: PathBuf,

    /// Paginate the listing, optionally up to N posts (no value or 0: no limit)
    #[arg(long, num_args = 0..=1, default_missing_value = "0", value_name = "N")]
    posts: Option<u64>,

    /// Listing order [default: hot, or the order in the URL]
    #[arg(long, value_enum, value_name = "ORDER")]
    sort: Option<Sort>,

    /// Time window for top/controversial [default: all, or the window in the URL]
    #[arg(long, value_enum, value_name = "WINDOW")]
    time: Option<TimeFilter>,

    /// Only keep fetched posts created on or after this UTC day
    #[arg(long, value_parser = parse_date, value_name = "YYYY-MM-DD")]
    since: Option<NaiveDate>,

    /// Also download videos
    #[arg(long)]
    videos: bool,

    /// Only download these formats (comma-separated or repeated)
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        ignore_case = true,
        value_name = "FORMAT"
    )]
    formats: Vec<MediaFormat>,

    /// Maximum images per gallery (0: all)
    #[arg(long, default_value_t = 0, value_name = "N")]
    gallery_images: usize,

    /// Skip stills whose short side is below W or long side below H
    #[arg(long, value_name = "WxH")]
    min_size: Option<Size>,

    /// Downscale stills to fit within this box
    #[arg(long, value_name = "WxH")]
    max_size: Option<Size>,

    /// Convert JPEG/PNG/BMP stills to this format (GIF/WebP are kept)
    #[arg(long, value_enum, ignore_case = true, value_name = "FORMAT")]
    convert: Option<ConvertFormat>,

    /// JPEG quality for converted or resized stills
    #[arg(long, default_value_t = 85, value_parser = clap::value_parser!(u8).range(1..=100), value_name = "1-100")]
    quality: u8,

    /// Skip the subreddit icon and banner
    #[arg(long)]
    no_icon: bool,

    /// Do not save the raw listing JSON
    #[arg(long)]
    no_raw: bool,

    /// Save JSON only; download no media
    #[arg(long)]
    no_downloads: bool,

    /// Write offline viewers and an archive hub (index.html)
    #[arg(long)]
    offline: bool,
}

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("expected YYYY-MM-DD: {e}"))
}

impl Cli {
    fn posts_mode(&self) -> PostsMode {
        match self.posts.map(NonZeroU64::new) {
            None => PostsMode::FirstPage,
            Some(None) => PostsMode::All,
            Some(Some(n)) => PostsMode::Limit(n),
        }
    }

    /// One config per target; explicit flags win over URL hints.
    fn configs(&self) -> Result<Vec<Config>> {
        // Fail on typos and unreadable cookies before any network request.
        let cookies = self
            .cookies
            .as_deref()
            .map(Cookies::from_arg)
            .transpose()?
            .unwrap_or_default();
        let mut formats = self.formats.clone();
        formats.sort();
        formats.dedup();

        self.targets
            .iter()
            .map(|raw| {
                let parsed = reddit::parse_target(raw)?;
                let mut config = Config::new(parsed.target);
                config.cookies = cookies.clone();
                config.user_agent = self.user_agent.clone();
                config.out_dir = self.out_dir.clone();
                config.posts = self.posts_mode();
                config.sort = self.sort.or(parsed.sort).unwrap_or_default();
                config.time = self.time.or(parsed.time).unwrap_or_default();
                config.since = self.since;
                config.media = Selection {
                    videos: self.videos,
                    formats: formats.clone(),
                    gallery_limit: (self.gallery_images > 0).then_some(self.gallery_images),
                    subreddit_art: !self.no_icon,
                    min_size: self.min_size,
                };
                config.transforms = TransformOptions {
                    max_size: self.max_size,
                    convert: self.convert,
                    quality: self.quality,
                };
                config.save_raw = !self.no_raw;
                config.download = !self.no_downloads;
                config.offline = self.offline;
                Ok(config)
            })
            .collect()
    }
}

fn print_summary(summaries: &[Summary]) {
    let posts: usize = summaries.iter().map(|s| s.posts).sum();
    let new: usize = summaries.iter().map(|s| s.posts_new).sum();
    let mut media = MediaReport::default();
    for summary in summaries {
        media += summary.media;
    }
    let notes: Vec<String> = [
        (media.converted, "converted"),
        (media.resized, "resized"),
        (media.skipped, "skipped as too small"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .collect();
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(" [{}]", notes.join(", "))
    };
    println!(
        "\ndone: {posts} posts ({new} new) | media: {} downloaded, {} cached, {} failed{notes}",
        media.downloaded, media.cached, media.failed
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let configs = cli.configs()?;
    let count = configs.len();
    let mut summaries = Vec::with_capacity(count);
    let mut failures = Vec::new();

    for (i, config) in configs.iter().enumerate() {
        if count > 1 {
            println!("\n=== {} ({}/{count}) ===", config.target, i + 1);
        }
        match reddit::run(config).await {
            Ok(summary) => summaries.push(summary),
            Err(e) => failures.push(format!("{}: {e:#}", config.target)),
        }
    }

    print_summary(&summaries);
    for failure in &failures {
        eprintln!("error: {failure}");
    }
    if !failures.is_empty() {
        bail!("{} of {count} archive(s) failed", failures.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("reddit").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn posts_flag_modes() {
        assert_eq!(parse(&["pics"]).posts_mode(), PostsMode::FirstPage);
        assert_eq!(parse(&["pics", "--posts"]).posts_mode(), PostsMode::All);
        assert_eq!(
            parse(&["pics", "--posts", "0"]).posts_mode(),
            PostsMode::All
        );
        assert_eq!(
            parse(&["pics", "--posts", "5"]).posts_mode(),
            PostsMode::Limit(NonZeroU64::new(5).unwrap())
        );
        // `--posts` without a value must not swallow the next target
        let cli = parse(&["pics", "--posts", "--offline", "rust"]);
        assert_eq!(cli.targets, ["pics", "rust"]);
        assert!(cli.offline);
    }

    #[test]
    fn formats_aliases_sizes_and_validation() {
        let cli = parse(&[
            "pics",
            "--formats",
            "jpeg,PNG",
            "--formats",
            "jpg",
            "--convert",
            "JPEG",
            "--min-size",
            "768x1024",
        ]);
        assert_eq!(
            cli.formats,
            [MediaFormat::Jpg, MediaFormat::Png, MediaFormat::Jpg]
        );
        assert_eq!(cli.convert, Some(ConvertFormat::Jpg));
        assert_eq!(cli.min_size, Size::new(768, 1024));
        let config = &cli.configs().unwrap()[0];
        assert_eq!(config.media.formats, [MediaFormat::Jpg, MediaFormat::Png]);

        for bad in [
            &["pics", "--quality", "0"][..],
            &["pics", "--quality", "101"],
            &["pics", "--max-size", "0x10"],
            &["pics", "--convert", "gif"],
            &["pics", "--since", "01-08-2026"],
        ] {
            assert!(
                Cli::try_parse_from(std::iter::once("reddit").chain(bad.iter().copied())).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn flags_override_url_hints() {
        let cli = parse(&[
            "https://www.reddit.com/r/rust/top/?t=week",
            "u/spez",
            "--time",
            "day",
        ]);
        let configs = cli.configs().unwrap();
        assert_eq!(
            (configs[0].sort, configs[0].time),
            (Sort::Top, TimeFilter::Day)
        );
        assert_eq!(
            (configs[1].sort, configs[1].time),
            (Sort::Hot, TimeFilter::Day)
        );
        assert!(parse(&["r/bad name"]).configs().is_err());
        assert!(
            parse(&["pics", "--cookies", "missing-cookies.txt"])
                .configs()
                .is_err()
        );
    }
}
