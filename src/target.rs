//! Listing targets: validation, parsing and URL hints.

use crate::models::{Sort, TimeFilter};
use anyhow::{Result, bail};
use std::fmt;

/// What a target points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// `r/<name>`: a subreddit or a `+`-joined multi (`rust+golang`).
    Subreddit,
    /// `u/<name>`: a user's submitted posts.
    User,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subreddit => "subreddit",
            Self::User => "user",
        }
    }

    /// Short prefix used for display (`r/rust`) and directories (`r_rust`).
    fn prefix(self) -> &'static str {
        match self {
            Self::Subreddit => "r",
            Self::User => "u",
        }
    }

    /// Reddit API path segment.
    fn api_segment(self) -> &'static str {
        match self {
            Self::Subreddit => "r",
            Self::User => "user",
        }
    }
}

/// A validated listing target. Names are restricted to Reddit's character
/// set, so they are safe to use in URLs and file names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    kind: TargetKind,
    name: String,
}

impl Target {
    pub fn new(kind: TargetKind, name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let valid = match kind {
            TargetKind::Subreddit => {
                name.len() <= 64
                    && name.split('+').all(|part| {
                        (1..=32).contains(&part.len())
                            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
            }
            TargetKind::User => {
                (2..=32).contains(&name.len())
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            }
        };
        if !valid {
            bail!("invalid {} name: '{name}'", kind.as_str());
        }
        Ok(Self { kind, name })
    }

    pub fn subreddit(name: impl Into<String>) -> Result<Self> {
        Self::new(TargetKind::Subreddit, name)
    }

    pub fn user(name: impl Into<String>) -> Result<Self> {
        Self::new(TargetKind::User, name)
    }

    pub fn kind(&self) -> TargetKind {
        self.kind
    }

    /// Subreddit name (may contain `+`) or username.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True for `r/a+b` multi-subreddit targets, which have no `about.json`.
    pub fn is_multi(&self) -> bool {
        self.kind == TargetKind::Subreddit && self.name.contains('+')
    }

    /// Directory name under the output root (`r_rust`, `u_spez`).
    pub fn dir_name(&self) -> String {
        format!("{}_{}", self.kind.prefix(), self.name)
    }

    /// API path without the `.json` suffix (`/r/rust`, `/user/spez`).
    pub fn api_path(&self) -> String {
        format!("/{}/{}", self.kind.api_segment(), self.name)
    }

    /// Canonical Reddit URL of the listing.
    pub fn url(&self) -> String {
        format!("https://www.reddit.com{}/", self.api_path())
    }
}

/// `r/rust` or `u/spez`.
impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.kind.prefix(), self.name)
    }
}

/// A target plus optional sort/time hints taken from its URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTarget {
    pub target: Target,
    pub sort: Option<Sort>,
    pub time: Option<TimeFilter>,
}

/// Parse names (`rust`), paths (`r/rust`, `u/spez`) and listing URLs
/// (`https://old.reddit.com/r/rust/top/?t=week`). A sort in the path wins over
/// a `sort=` query hint.
pub fn parse_target(input: &str) -> Result<ParsedTarget> {
    let raw = input.trim();
    let (before_query, query) = raw.split_once('?').unwrap_or((raw, ""));
    let query = query.split('#').next().unwrap_or_default();
    let location = before_query.split('#').next().unwrap_or_default();
    let path = match location.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => match location.split_once('/') {
            Some((host, path)) if host.contains('.') => path,
            _ => location,
        },
    };

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let (kind, name, rest) = match segments.as_slice() {
        [] => bail!("no subreddit or user in '{input}'"),
        [prefix] if is_prefix(prefix) => bail!("missing name in '{input}'"),
        [prefix, name, rest @ ..] if is_prefix(prefix) => {
            let kind = if prefix.eq_ignore_ascii_case("r") {
                TargetKind::Subreddit
            } else {
                TargetKind::User
            };
            (kind, *name, rest)
        }
        [name, rest @ ..] => (TargetKind::Subreddit, *name, rest),
    };
    let target = Target::new(kind, name)?;

    let mut sort = rest.first().and_then(|segment| Sort::from_name(segment));
    let mut time = None;
    for (key, value) in query.split('&').filter_map(|pair| pair.split_once('=')) {
        if key.eq_ignore_ascii_case("sort") {
            sort = sort.or_else(|| Sort::from_name(value));
        } else if key.eq_ignore_ascii_case("t") {
            time = TimeFilter::from_name(value);
        }
    }
    Ok(ParsedTarget { target, sort, time })
}

fn is_prefix(segment: &str) -> bool {
    ["r", "u", "user"]
        .iter()
        .any(|prefix| segment.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(input: &str) -> Target {
        parse_target(input).unwrap().target
    }

    #[test]
    fn names_paths_and_urls() {
        let rust = Target::subreddit("rust").unwrap();
        let spez = Target::user("spez").unwrap();
        for input in [
            "rust",
            "r/rust",
            "/r/rust/",
            "R/rust",
            "reddit.com/r/rust",
            "https://www.reddit.com/r/rust",
            "https://www.reddit.com/r/rust/comments/abc123/some_title/",
            "https://www.reddit.com/r/rust#top",
        ] {
            assert_eq!(target(input), rust, "{input}");
        }
        for input in [
            "u/spez",
            "user/spez/submitted",
            "https://www.reddit.com/user/spez",
        ] {
            assert_eq!(target(input), spez, "{input}");
        }
    }

    #[test]
    fn sort_and_time_hints() {
        let parsed = parse_target("https://www.reddit.com/r/rust").unwrap();
        assert_eq!((parsed.sort, parsed.time), (None, None));

        let parsed = parse_target("https://old.reddit.com/r/rust/top/?t=week").unwrap();
        assert_eq!(parsed.sort, Some(Sort::Top));
        assert_eq!(parsed.time, Some(TimeFilter::Week));

        let parsed = parse_target("https://www.reddit.com/r/rust/new?sort=controversial").unwrap();
        assert_eq!(parsed.sort, Some(Sort::New), "path hint wins over query");

        let parsed = parse_target("r/rust?sort=TOP&t=Day").unwrap();
        assert_eq!(parsed.sort, Some(Sort::Top));
        assert_eq!(parsed.time, Some(TimeFilter::Day));
    }

    #[test]
    fn multi_subreddits() {
        let multi = target("r/rust+golang/new");
        assert_eq!(multi.name(), "rust+golang");
        assert!(multi.is_multi());
        assert_eq!(multi.dir_name(), "r_rust+golang");
    }

    #[test]
    fn invalid_targets_are_rejected() {
        for bad in [
            "",
            "https://www.reddit.com/",
            "r/",
            "r/bad name!",
            "r/../etc",
            "r/a++b",
            "u/x",
            "u/",
        ] {
            assert!(parse_target(bad).is_err(), "should reject {bad:?}");
        }
        assert!(Target::subreddit("").is_err());
        assert!(Target::subreddit("a".repeat(33)).is_err());
    }

    #[test]
    fn display_paths_and_urls() {
        let funny = Target::subreddit("funny").unwrap();
        assert_eq!(funny.to_string(), "r/funny");
        assert_eq!(funny.dir_name(), "r_funny");
        assert_eq!(funny.api_path(), "/r/funny");
        assert_eq!(funny.url(), "https://www.reddit.com/r/funny/");
        let spez = Target::user("spez").unwrap();
        assert_eq!(spez.to_string(), "u/spez");
        assert_eq!(spez.dir_name(), "u_spez");
        assert_eq!(spez.url(), "https://www.reddit.com/user/spez/");
    }
}
