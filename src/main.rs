use anyhow::Result;
use clap::{ArgAction, Parser, ValueEnum};
use reddit::{Config, PostsMode, Sort, TimeFilter, UA_DEFAULT};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "reddit",
    version,
    about = "Reddit offline viewer: save a subreddit's posts and media as JSON",
    long_about = "Offline viewer for reddit. By default it saves the first page of a \
listing (~25 posts) as JSON and downloads their images — galleries keep every image, \
videos are skipped. Use --posts 0 to paginate the whole listing and --videos to grab \
reddit-hosted video files.\n\n\
Pass --cookies with a Netscape cookies.txt (or 'k=v; k2=v2'). Reddit rejects \
anonymous JSON requests from many networks, so cookies exported from your browser \
are the reliable way to fetch any listing — they also unlock private and NSFW \
subreddits, provided the account can view them.\n\n\
Examples:\n  \
reddit funny --cookies cookies.txt\n  \
reddit https://www.reddit.com/r/funny --sort new --posts 5 --cookies cookies.txt\n  \
reddit rust --posts 0 --offline --cookies cookies.txt\n  \
reddit u/spez --sort top --time year --gallery-images 10 --cookies cookies.txt"
)]
struct Cli {
    /// Subreddit or user: name, r/name, u/name or a full reddit URL
    #[arg(value_name = "SUBREDDIT_OR_URL")]
    target: String,

    /// Netscape cookies.txt path or 'k=v; k2=v2' string
    /// (recommended: reddit rejects anonymous JSON requests; also unlocks private/NSFW)
    #[arg(long)]
    cookies: Option<String>,

    /// must match the browser the cookies were exported from
    #[arg(long, default_value = UA_DEFAULT)]
    user_agent: String,

    /// output directory (default: output/<r_|u_><name>)
    #[arg(long, default_value = "output", value_name = "DIR")]
    out_dir: PathBuf,

    /// paginate the listing; optional cap: --posts 50 (0 = everything)
    #[arg(long, num_args = 0..=1, default_missing_value = "0", value_name = "N")]
    posts: Option<Option<u64>>,

    /// listing order [default: hot, or the order in the URL]
    #[arg(long, value_enum, value_name = "ORDER")]
    sort: Option<SortArg>,

    /// time window for top/controversial [default: all]
    #[arg(long, value_enum, value_name = "WINDOW")]
    time: Option<TimeArg>,

    /// also download reddit-hosted video files (images only by default)
    #[arg(long, action = ArgAction::SetTrue)]
    videos: bool,

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

    /// generate a self-contained index.html to browse the archive offline
    #[arg(long, action = ArgAction::SetTrue)]
    offline: bool,
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
    let parsed = reddit::parse_target(&cli.target)?;

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
    let posts = match cli.posts {
        Some(Some(n)) => PostsMode::All(n),
        Some(None) => PostsMode::All(0),
        None => PostsMode::Snapshot,
    };

    let cfg = Config {
        target: parsed.target,
        cookies: cli.cookies,
        user_agent: cli.user_agent,
        out_dir: cli.out_dir,
        posts,
        sort,
        time,
        videos: cli.videos,
        gallery_images: cli.gallery_images,
        since: cli.since,
        skip_icon: cli.no_icon,
        no_raw: cli.no_raw,
        no_downloads: cli.no_downloads,
        offline: cli.offline,
        ..Config::default()
    };

    let summary = reddit::run(cfg).await?;
    println!(
        "\ndone — {} posts | media: {} downloaded ({} failed)",
        summary.posts, summary.media_total, summary.media_failed
    );
    Ok(())
}
