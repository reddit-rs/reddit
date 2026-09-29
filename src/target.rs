//! Listing target validation and URL hints.

use crate::models::{Sort, Target, TargetKind, TimeFilter};
use anyhow::{Result, bail};

/// A validated listing target and optional sort/time hints from its URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTarget {
    pub target: Target,
    pub sort: Option<Sort>,
    pub time: Option<TimeFilter>,
}

pub(crate) fn validate_target(target: &Target) -> Result<()> {
    let valid = match target.kind {
        TargetKind::Subreddit => {
            !target.name.is_empty()
                && target.name.len() <= 64
                && target.name.split('+').all(|part| {
                    !part.is_empty()
                        && part.len() <= 32
                        && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
        }
        TargetKind::User => {
            (2..=32).contains(&target.name.len())
                && target
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }
    };
    if !valid {
        bail!("invalid {} name: '{}'", target.kind.name(), target.name);
    }
    Ok(())
}

/// Parse names (`rust`), paths (`r/rust`, `u/spez`), and listing URLs.
/// Multi-subreddits (`r/rust+golang`) are supported; path sort hints take
/// precedence over query hints.
pub fn parse_target(s: &str) -> Result<ParsedTarget> {
    let raw = s.trim();
    if raw.is_empty() {
        bail!("no subreddit or user given");
    }
    let (before_query, query) = raw.split_once('?').unwrap_or((raw, ""));
    let clean = before_query.trim_end_matches('/');
    let path = if clean.starts_with("http://") || clean.starts_with("https://") {
        let rest = clean.split_once("://").map(|(_, r)| r).unwrap_or(clean);
        rest.find('/').map(|i| &rest[i..]).unwrap_or("")
    } else if let Some((host, rest)) = clean.split_once('/')
        && host.contains('.')
    {
        rest
    } else {
        clean
    };

    let segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (kind, name, rest): (TargetKind, &str, &[&str]) = match segs.as_slice() {
        ["r" | "R", name, rest @ ..] => (TargetKind::Subreddit, name, rest),
        ["u" | "U" | "user" | "User", name, rest @ ..] => (TargetKind::User, name, rest),
        ["r" | "R" | "u" | "U" | "user" | "User"] => bail!("missing name in '{s}'"),
        [name, rest @ ..] => (TargetKind::Subreddit, name, rest),
        [] => bail!("no subreddit or user in '{s}'"),
    };
    let target = Target {
        kind,
        name: name.to_string(),
    };
    validate_target(&target)?;

    let mut sort = rest
        .first()
        .and_then(|s| Sort::from_name(&s.to_lowercase()));
    let mut time = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = v.to_lowercase();
        match k.to_lowercase().as_str() {
            "sort" => sort = sort.or_else(|| Sort::from_name(&v)),
            "t" => time = TimeFilter::from_name(&v),
            _ => {}
        }
    }
    Ok(ParsedTarget { target, sort, time })
}
