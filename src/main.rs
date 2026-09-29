use anyhow::Result;
use clap::{ArgAction, Parser, ValueEnum};
use reddit::{Config, MediaFormat, PostsMode, Sort, TimeFilter, UA_DEFAULT};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "reddit",
    version,
    about = "Reddit offline viewer: save subreddit posts and media as JSON",
    long_about = "Archive subreddit or user listings as JSON and media. Defaults to \
the first listing page (~25 posts), all gallery images, and no videos. Re-runs \
merge posts and reuse cached media. Reddit may require browser-exported cookies.",
    after_help = "Examples:\n  \
reddit pics --offline --cookies cookies.txt\n  \
reddit pics rust --posts 0 --offline --cookies cookies.txt\n  \
reddit gifs --formats gif --cookies cookies.txt"
)]
struct Cli {
    /// One or more subreddits/users: name, r/name, u/name or a full reddit URL
    #[arg(value_name = "SUBREDDIT_OR_URL", required = true, num_args = 1..)]
    targets: Vec<String>,

    /// Netscape cookies.txt path or 'k=v; k2=v2' string
    /// (recommended: reddit rejects anonymous JSON requests; also unlocks private/NSFW)
    #[arg(long)]
    cookies: Option<String>,

    /// must match the browser the cookies were exported from
    #[arg(long, default_value = UA_DEFAULT)]
    user_agent: String,

    /// output directory (default: output/<r_|u_><name>, or $REDDIT_OUT_DIR)
    #[arg(
        long,
        default_value = "output",
        env = "REDDIT_OUT_DIR",
        value_name = "DIR"
    )]
    out_dir: PathBuf,

    /// paginate the listing; optional cap: --posts 50 (0 = everything)
    #[arg(long, num_args = 0..=1, default_missing_value = "0", value_name = "N")]
    posts: Option<Option<u64>>,

    /// listing order [default: hot, or the order in each URL]
    #[arg(long, value_enum, value_name = "ORDER")]
    sort: Option<SortArg>,

    /// time window for top/controversial [default: all]
    #[arg(long, value_enum, value_name = "WINDOW")]
    time: Option<TimeArg>,

    /// also download reddit-hosted video files (images only by default)
    #[arg(long, action = ArgAction::SetTrue)]
    videos: bool,

    /// only download these media formats (repeatable or comma-separated;
    /// mp4/webm imply --videos)
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        ignore_case = true,
        value_name = "FORMAT"
    )]
    formats: Vec<FormatArg>,

    /// skip still images smaller than this: short side < W or long side < H
    #[arg(long, value_parser = parse_size, value_name = "WxH")]
    min_size: Option<(u32, u32)>,

    /// downscale still images so neither side exceeds this box
    #[arg(long, value_parser = parse_size, value_name = "WxH")]
    max_size: Option<(u32, u32)>,

    /// convert JPEG/PNG/BMP stills to this format (gif/webp are kept as-is)
    #[arg(long, value_enum, ignore_case = true, value_name = "FORMAT")]
    convert: Option<ConvertArg>,

    /// JPEG quality for converted/resized images (1-100)
    #[arg(
        long,
        default_value_t = 85,
        value_parser = clap::value_parser!(u8).range(1..=100),
        value_name = "N"
    )]
    quality: u8,

    /// max images per gallery (0 = all)
    #[arg(long, default_value_t = 0, value_name = "N")]
    gallery_images: usize,

    /// only posts created on/after this date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    since: Option<String>,

    /// skip the subreddit icon and banner
    #[arg(long, action = ArgAction::SetTrue)]
    no_icon: bool,

    /// skip the raw listing JSON dump
    #[arg(long, action = ArgAction::SetTrue)]
    no_raw: bool,

    /// skip media downloads entirely (JSON only)
    #[arg(long, action = ArgAction::SetTrue)]
    no_downloads: bool,

    /// generate self-contained index.html viewers (plus an archive hub)
    #[arg(long, action = ArgAction::SetTrue)]
    offline: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    /// JPEG images (`jpeg` is accepted too)
    #[value(alias = "jpeg")]
    Jpg,
    Png,
    Gif,
    Webp,
    Bmp,
    /// MP4 video (implies --videos)
    Mp4,
    /// WebM video (implies --videos)
    Webm,
}

impl From<FormatArg> for MediaFormat {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Jpg => MediaFormat::Jpg,
            FormatArg::Png => MediaFormat::Png,
            FormatArg::Gif => MediaFormat::Gif,
            FormatArg::Webp => MediaFormat::Webp,
            FormatArg::Bmp => MediaFormat::Bmp,
            FormatArg::Mp4 => MediaFormat::Mp4,
            FormatArg::Webm => MediaFormat::Webm,
        }
    }
}

/// Conversion targets; GIF/WebP are excluded because they may be animated.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ConvertArg {
    /// JPEG (`jpeg` is accepted too; transparency is flattened onto white)
    #[value(alias = "jpeg")]
    Jpg,
    Png,
}

impl From<ConvertArg> for MediaFormat {
    fn from(c: ConvertArg) -> Self {
        match c {
            ConvertArg::Jpg => MediaFormat::Jpg,
            ConvertArg::Png => MediaFormat::Png,
        }
    }
}

/// `WxH` with both sides greater than zero.
fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("invalid size '{s}': expected WxH, e.g. 768x1024");
    let split = s.find(['x', 'X']).ok_or_else(bad)?;
    let (w, h) = (&s[..split], &s[split + 1..]);
    let w: u32 = w.trim().parse().map_err(|_| bad())?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    if w == 0 || h == 0 {
        return Err(format!(
            "invalid size '{s}': sides must be greater than zero"
        ));
    }
    Ok((w, h))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum SortArg {
    Hot,
    New,
    Top,
    Rising,
    Controversial,
}

impl From<SortArg> for Sort {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::Hot => Sort::Hot,
            SortArg::New => Sort::New,
            SortArg::Top => Sort::Top,
            SortArg::Rising => Sort::Rising,
            SortArg::Controversial => Sort::Controversial,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum TimeArg {
    Hour,
    Day,
    Week,
    Month,
    Year,
    All,
}

impl From<TimeArg> for TimeFilter {
    fn from(t: TimeArg) -> Self {
        match t {
            TimeArg::Hour => TimeFilter::Hour,
            TimeArg::Day => TimeFilter::Day,
            TimeArg::Week => TimeFilter::Week,
            TimeArg::Month => TimeFilter::Month,
            TimeArg::Year => TimeFilter::Year,
            TimeArg::All => TimeFilter::All,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Parse every target up front: a typo should fail before any network call.
    let parsed: Vec<reddit::ParsedTarget> = cli
        .targets
        .iter()
        .map(|raw| reddit::parse_target(raw))
        .collect::<Result<_>>()?;

    let posts = match cli.posts {
        Some(Some(n)) => PostsMode::All(n),
        Some(None) => PostsMode::All(0),
        None => PostsMode::Snapshot,
    };

    // `--formats jpeg` and `--formats jpg` are the same request.
    let mut formats: Vec<MediaFormat> =
        cli.formats.iter().copied().map(MediaFormat::from).collect();
    formats.sort();
    formats.dedup();

    let multiple = parsed.len() > 1;
    let mut summaries = Vec::with_capacity(parsed.len());
    let mut failures = Vec::new();

    for (i, parsed) in parsed.into_iter().enumerate() {
        // explicit flags win over hints embedded in the URL
        let sort = cli
            .sort
            .map(Sort::from)
            .or(parsed.sort)
            .unwrap_or(Sort::Hot);
        let time = cli
            .time
            .map(TimeFilter::from)
            .or(parsed.time)
            .unwrap_or(TimeFilter::All);

        if multiple {
            println!(
                "\n=== {} ({}/{}) ===",
                parsed.target.display(),
                i + 1,
                cli.targets.len()
            );
        }

        let cfg = Config {
            target: parsed.target,
            cookies: cli.cookies.clone(),
            user_agent: cli.user_agent.clone(),
            out_dir: cli.out_dir.clone(),
            posts,
            sort,
            time,
            videos: cli.videos,
            formats: formats.clone(),
            min_size: cli.min_size,
            max_size: cli.max_size,
            convert: cli.convert.map(MediaFormat::from),
            quality: cli.quality,
            gallery_images: cli.gallery_images,
            since: cli.since.clone(),
            skip_icon: cli.no_icon,
            no_raw: cli.no_raw,
            no_downloads: cli.no_downloads,
            offline: cli.offline,
            ..Config::default()
        };

        let label = cfg.target.display();
        match reddit::run(cfg).await {
            Ok(summary) => summaries.push(summary),
            Err(e) => failures.push(format!("{label}: {e}")),
        }
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("error: {f}");
        }
        anyhow::bail!(
            "{} of {} archive(s) failed",
            failures.len(),
            cli.targets.len()
        );
    }

    let total = |f: fn(&reddit::Summary) -> usize| summaries.iter().map(f).sum::<usize>();
    let (posts, new, downloaded) = (
        total(|s| s.posts),
        total(|s| s.posts_new),
        total(|s| s.media_downloaded),
    );
    let mut notes = Vec::new();
    for (count, label) in [
        (total(|s| s.media_converted), "converted"),
        (total(|s| s.media_resized), "resized"),
        (total(|s| s.media_skipped), "skipped small"),
    ] {
        if count > 0 {
            notes.push(format!("{count} {label}"));
        }
    }
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(" [{}]", notes.join(", "))
    };
    println!(
        "\ndone — {posts} posts ({new} new) | media: {downloaded} downloaded, {} cached, {} failed{notes}",
        total(|s| s.media_cached),
        total(|s| s.media_failed),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn sizes_require_two_positive_dimensions() {
        assert_eq!(parse_size("768x1024").unwrap(), (768, 1024));
        assert_eq!(parse_size(" 768X1024 ").unwrap(), (768, 1024));
        for value in [
            "",
            "768",
            "0x1024",
            "768x0",
            "1x2x3",
            "-1x2",
            "4294967296x1",
        ] {
            assert!(parse_size(value).is_err(), "{value}");
        }
    }

    #[test]
    fn optional_post_caps_and_multiple_targets() {
        let cli = Cli::try_parse_from(["reddit", "pics", "rust", "--posts", "--offline"]).unwrap();
        assert_eq!(cli.targets, ["pics", "rust"]);
        assert_eq!(cli.posts, Some(Some(0)));
        assert!(cli.offline);
        assert_eq!(
            Cli::try_parse_from(["reddit", "pics", "--posts", "5"])
                .unwrap()
                .posts,
            Some(Some(5))
        );
    }

    #[test]
    fn format_aliases_and_quality_validation() {
        let cli = Cli::try_parse_from([
            "reddit",
            "pics",
            "--formats",
            "jpeg,png",
            "--convert",
            "JPEG",
        ])
        .unwrap();
        assert_eq!(cli.formats, [FormatArg::Jpg, FormatArg::Png]);
        assert_eq!(cli.convert, Some(ConvertArg::Jpg));
        for quality in ["0", "101"] {
            assert!(Cli::try_parse_from(["reddit", "pics", "--quality", quality]).is_err());
        }
    }
}
