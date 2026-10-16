//! Reddit's JSON API: listings and subreddit metadata.

use crate::cookies::Cookies;
use crate::http;
use crate::models::{Sort, SubredditInfo, TimeFilter};
use crate::normalize;
use crate::target::{Target, TargetKind};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::collections::HashSet;
use std::num::NonZeroU64;
use std::time::Duration;
use wreq::StatusCode;
use wreq::header::{ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, REFERER};

/// Safety net for listings that keep returning cursors.
pub const MAX_PAGES: usize = 200;
/// Reddit's maximum page size.
const MAX_PAGE_SIZE: u64 = 100;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// How much of a listing to fetch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostsMode {
    /// The first page, as a browser shows it (25 posts).
    #[default]
    FirstPage,
    /// Paginate until this many posts are collected.
    Limit(NonZeroU64),
    /// Paginate until the listing ends (Reddit caps history at ~1000 posts).
    All,
}

impl PostsMode {
    fn page_size(self) -> u64 {
        match self {
            Self::FirstPage => 25,
            Self::Limit(n) => n.get().min(MAX_PAGE_SIZE),
            Self::All => MAX_PAGE_SIZE,
        }
    }

    fn max_pages(self) -> usize {
        match self {
            Self::FirstPage => 1,
            Self::Limit(_) | Self::All => MAX_PAGES,
        }
    }

    fn cap(self) -> Option<usize> {
        match self {
            Self::Limit(n) => Some(usize::try_from(n.get()).unwrap_or(usize::MAX)),
            Self::FirstPage | Self::All => None,
        }
    }
}

/// A listing request.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Listing<'a> {
    pub target: &'a Target,
    pub sort: Sort,
    pub time: TimeFilter,
    pub posts: PostsMode,
}

pub(crate) struct Api {
    http: wreq::Client,
    base: String,
    headers: HeaderMap,
    page_delay: Duration,
}

impl Api {
    /// `over18=1` is added unless supplied: NSFW listings answer 403 without
    /// the opt-in, and it is harmless elsewhere.
    pub fn new(http: wreq::Client, base: &str, cookies: &Cookies, page_delay: Duration) -> Self {
        let mut cookies = cookies.clone();
        cookies.insert_default("over18", "1");

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        if let Some(cookie) = cookies.header_value() {
            headers.insert(COOKIE, cookie);
        }
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            headers,
            page_delay,
        }
    }

    async fn get_json(
        &self,
        target: &Target,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let mut headers = self.headers.clone();
        if let Ok(referer) = HeaderValue::from_str(&format!("{}{}/", self.base, target.api_path()))
        {
            headers.insert(REFERER, referer);
        }

        let mut last_error = anyhow!("no request made");
        for attempt in 0..http::ATTEMPTS {
            let response = match self
                .http
                .get(&url)
                .headers(headers.clone())
                .query(query)
                .timeout(REQUEST_TIMEOUT)
                .send()
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    last_error = anyhow!("network error: {e}");
                    tokio::time::sleep(http::backoff(attempt)).await;
                    continue;
                }
            };
            match response.status() {
                StatusCode::OK => {
                    let budget_wait = http::budget_exhausted(&response);
                    match response.json::<Value>().await {
                        Ok(value) => {
                            if let Some(wait) = budget_wait {
                                println!(
                                    "  rate limit budget used up, waiting {}s",
                                    wait.as_secs()
                                );
                                tokio::time::sleep(wait).await;
                            }
                            return Ok(value);
                        }
                        // Reddit serves HTML block pages with status 200.
                        Err(e) => last_error = anyhow!("response is not JSON (blocked?): {e}"),
                    }
                }
                StatusCode::FORBIDDEN => bail!(
                    "HTTP 403 for {path}: access denied. The listing may be private, \
                     quarantined or NSFW, or Reddit rejected the session; pass --cookies \
                     from a signed-in browser (and its --user-agent)."
                ),
                StatusCode::NOT_FOUND => bail!("HTTP 404 for {path}: not found"),
                StatusCode::TOO_MANY_REQUESTS => {
                    let wait = http::rate_limit_wait(response.headers(), attempt);
                    last_error = anyhow!("HTTP 429 for {path}: rate limited");
                    if attempt + 1 < http::ATTEMPTS {
                        println!("  rate limited, waiting {}s", wait.as_secs());
                        tokio::time::sleep(wait).await;
                    }
                    continue;
                }
                status if status.is_server_error() => {
                    last_error = anyhow!("HTTP {status} for {path}");
                }
                status => bail!("HTTP {status} for {path}"),
            }
            if attempt + 1 < http::ATTEMPTS {
                tokio::time::sleep(http::backoff(attempt)).await;
            }
        }
        Err(last_error)
            .with_context(|| format!("giving up on {path} after {} attempts", http::ATTEMPTS))
    }

    /// Subreddit metadata (`/r/<name>/about.json`).
    pub async fn about(&self, target: &Target) -> Result<SubredditInfo> {
        let path = format!("{}/about.json", target.api_path());
        let value = self
            .get_json(target, &path, &[("raw_json", "1".into())])
            .await?;
        Ok(normalize::subreddit(&value))
    }

    /// Fetch a listing; returns the raw `t3` data objects, deduplicated, in
    /// listing order.
    pub async fn listing(&self, request: Listing<'_>) -> Result<Vec<Value>> {
        let target = request.target;
        let path = match target.kind() {
            TargetKind::Subreddit => {
                format!("{}/{}.json", target.api_path(), request.sort.as_str())
            }
            TargetKind::User => format!("{}/submitted.json", target.api_path()),
        };
        let cap = request.posts.cap();
        let mut items: Vec<Value> = Vec::new();
        let mut seen = HashSet::new();
        let mut after: Option<String> = None;

        for page in 1..=request.posts.max_pages() {
            let mut query = vec![
                ("limit", request.posts.page_size().to_string()),
                ("raw_json", "1".to_owned()),
                ("count", items.len().to_string()),
            ];
            if let Some(cursor) = &after {
                query.push(("after", cursor.clone()));
            }
            if target.kind() == TargetKind::User {
                query.push(("sort", request.sort.as_str().to_owned()));
            }
            if request.sort.uses_time() {
                query.push(("t", request.time.as_str().to_owned()));
            }

            let mut data = self.get_json(target, &path, &query).await?;
            if let Some(message) = data
                .get("message")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
            {
                bail!("Reddit error for {target}: {message}");
            }
            let children = match data.pointer_mut("/data/children").map(Value::take) {
                Some(Value::Array(children)) => children,
                _ => bail!("unexpected listing response for {target}: no data.children"),
            };
            let before = items.len();
            for mut child in children {
                if child.get("kind").and_then(Value::as_str) != Some("t3") {
                    continue;
                }
                let post = child["data"].take();
                if let Some(id) = post.get("id").and_then(Value::as_str)
                    && !id.is_empty()
                    && seen.insert(id.to_owned())
                {
                    items.push(post);
                }
            }
            let added = items.len() - before;
            println!("  page {page}: +{added} posts ({} total)", items.len());

            after = data
                .pointer("/data/after")
                .and_then(Value::as_str)
                .filter(|a| !a.is_empty())
                .map(str::to_owned);
            if cap.is_some_and(|cap| items.len() >= cap) || after.is_none() || added == 0 {
                break;
            }
            if page == request.posts.max_pages() {
                if request.posts != PostsMode::FirstPage {
                    eprintln!("warning: stopped after {MAX_PAGES} pages");
                }
            } else if !self.page_delay.is_zero() {
                tokio::time::sleep(self.page_delay).await;
            }
        }

        if let Some(cap) = cap {
            items.truncate(cap);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_modes_map_to_pages() {
        let five = PostsMode::Limit(NonZeroU64::new(5).unwrap());
        let many = PostsMode::Limit(NonZeroU64::new(500).unwrap());
        assert_eq!(
            [PostsMode::FirstPage, five, many, PostsMode::All].map(PostsMode::page_size),
            [25, 5, 100, 100]
        );
        assert_eq!(PostsMode::FirstPage.max_pages(), 1);
        assert_eq!(PostsMode::All.max_pages(), MAX_PAGES);
        assert_eq!(many.cap(), Some(500));
        assert_eq!(PostsMode::All.cap(), None);
    }
}
