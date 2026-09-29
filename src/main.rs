use anyhow::Result;
use clap::{ArgAction, Parser, ValueEnum};
use reddit::{Config, MediaFormat, PostsMode, Sort, TimeFilter, UA_DEFAULT};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "reddit",
    version,
    about = "Reddit offline viewer: save subreddit posts and media as JSON",
    long_about = "Offline viewer for reddit. By default it saves the first page of a \
listing (~25 posts) as JSON and downloads their images — galleries keep every image, \
videos are skipped. Use --posts 0 to paginate the whole listing and --videos to grab \
reddit-hosted video files.\n\n\
Only some media containers can be kept: --formats gif downloads just the GIFs, \
--formats jpg,jpeg keeps JPEG stills (jpg and jpeg are the same format). The listing \
JSON still describes every post. Video formats imply --videos.\n\n\
Several subreddits (or users) can be archived in one run; each gets its own directory \
under the output root. Re-running a target is incremental: the fresh listing is merged \
into the existing archive and media files already on disk are not downloaded again.\n\n\
Pass --cookies with a Netscape cookies.txt (or 'k=v; k2=v2'). Reddit rejects \
anonymous JSON requests from many networks, so cookies exported from your browser \
are the reliable way to fetch any listing — they also unlock private and NSFW \
subreddits, provided the account can view them.\n\n\
Examples:\n  \
reddit funny --cookies cookies.txt\n  \
reddit funny rust --posts 0 --offline --cookies cookies.txt\n  \
reddit gifs --formats gif --posts 0 --cookies cookies.txt\n  \
reddit https://www.reddit.com/r/funny --sort new --posts 5 --cookies cookies.txt\n  \
reddit u/spez --sort top --time year --gallery-images 10 --cookies cookies.txt"
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
    println!(
        "\ndone — {} posts ({} new) | media: {} downloaded, {} cached, {} failed",
        total(|s| s.posts),
        total(|s| s.posts_new),
        total(|s| s.media_downloaded),
        total(|s| s.media_cached),
        total(|s| s.media_failed),
    );
    Ok(())
}
