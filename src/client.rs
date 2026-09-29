//! HTTP client for reddit's public JSON API (`www.reddit.com/<listing>.json`).

use crate::clean::get_str;
use crate::models::{Sort, Target, TargetKind, TimeFilter};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use wreq::Client as HttpClient;
use wreq::header::{HeaderMap, HeaderValue};
use wreq_util::Emulation;

/// Safety net for pagination when reddit keeps returning cursors.
pub const MAX_PAGES: usize = 200;
const RETRIES: u64 = 4;

pub struct Client {
    client: HttpClient,
    base: String,
    cookie_header: Option<String>,
    /// Delay between paginated listing requests.
    pub page_delay_ms: u64,
}

impl Client {
    pub fn new(ua: &str, cookies: &HashMap<String, String>) -> Result<Self> {
        Self::with_base(ua, cookies, "https://www.reddit.com")
    }

    pub fn with_base(ua: &str, cookies: &HashMap<String, String>, base: &str) -> Result<Self> {
        let mut builder = HttpClient::builder()
            .emulation(Emulation::Chrome131)
            .timeout(Duration::from_secs(60));
        if !ua.is_empty() {
            builder = builder.user_agent(ua);
        }
        let client = builder.build().context("failed to build HTTP client")?;

        // NSFW subreddits answer 403 unless the session opted in; the cookie is
        // harmless for regular listings.
        let mut all = cookies.clone();
        all.entry("over18".to_string())
            .or_insert_with(|| "1".to_string());
        let cookie_header = if all.is_empty() {
            None
        } else {
            let mut pairs: Vec<(String, String)> = all.into_iter().collect();
            pairs.sort();
            Some(
                pairs
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };

        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_string(),
            cookie_header,
            page_delay_ms: 1200,
        })
    }

    /// Session header for API requests and trusted Reddit media URLs (never logged).
    pub fn cookie_header(&self) -> Option<&str> {
        self.cookie_header.as_deref()
    }

    fn headers(&self, referer: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in [
            ("Accept", "application/json"),
            ("Accept-Language", "en-US,en;q=0.9"),
        ] {
            h.insert(k, HeaderValue::from_static(v));
        }
        if !referer.is_empty()
            && let Ok(v) = HeaderValue::from_str(referer)
        {
            h.insert("Referer", v);
        }
        if let Some(c) = &self.cookie_header
            && let Ok(mut v) = HeaderValue::from_str(c)
        {
            v.set_sensitive(true);
            h.insert("Cookie", v);
        }
        h
    }

    async fn get_json(
        &self,
        path: &str,
        params: &[(&str, String)],
        referer: &str,
    ) -> Result<Value> {
        let mut last_err = anyhow::anyhow!("no attempt made");
        for attempt in 0..RETRIES {
            let url = format!("{}{}", self.base, path);
            let mut req = self.client.get(&url).headers(self.headers(referer));
            if !params.is_empty() {
                req = req.query(params);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    last_err = anyhow::anyhow!("network error: {e}");
                    tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                    continue;
                }
            };
            match resp.status().as_u16() {
                200 => match resp.json::<Value>().await {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        last_err = anyhow::anyhow!("non-JSON response: {e}");
                        println!("  non-JSON response (likely blocked), retrying...");
                        tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                        continue;
                    }
                },
                403 => bail!(
                    "HTTP 403 for {path}: access denied — the listing may be private, \
                     quarantined or NSFW. Pass --cookies from an account that can view it."
                ),
                404 => bail!("HTTP 404 for {path}: not found"),
                429 => {
                    last_err = anyhow::anyhow!("HTTP 429 for {path}: rate limited");
                    let wait = 15 * (attempt + 1);
                    println!("  rate limited (429), waiting {wait}s...");
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                500..=599 => {
                    last_err = anyhow::anyhow!("HTTP {} for {path}", resp.status());
                    tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                    continue;
                }
                other => bail!("HTTP {other} for {path}"),
            }
        }
        Err(last_err.context(format!("request failed after retries: {path}")))
    }

    /// `/r/<name>/about.json` — subreddit metadata.
    pub async fn get_about(&self, target: &Target) -> Result<Value> {
        let path = format!("/r/{}/about.json", target.name);
        let referer = format!("{}/{}/", self.base, target.display());
        self.get_json(&path, &[("raw_json", "1".to_string())], &referer)
            .await
    }

    /// Paginate a listing until `cap` posts are collected (`None` = everything)
    /// or reddit stops returning a cursor. Returns raw `t3` data objects.
    pub async fn get_listing(
        &self,
        target: &Target,
        sort: Sort,
        time: TimeFilter,
        cap: Option<u64>,
        page_size: u64,
        max_pages: usize,
    ) -> Result<Vec<Value>> {
        let path = match target.kind {
            TargetKind::Subreddit => format!("/r/{}/{}.json", target.name, sort.name()),
            TargetKind::User => format!("/user/{}/submitted.json", target.name),
        };
        let referer = format!("{}/{}/", self.base, target.display());

        let mut items: Vec<Value> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut after: Option<String> = None;
        let mut page = 0usize;

        while page < max_pages {
            let mut params: Vec<(&str, String)> = vec![
                ("limit", page_size.to_string()),
                ("raw_json", "1".to_string()),
                ("count", items.len().to_string()),
            ];
            if let Some(a) = &after {
                params.push(("after", a.clone()));
            }
            if sort.uses_time() {
                params.push(("t", time.name().to_string()));
            }
            if target.kind == TargetKind::User {
                params.push(("sort", sort.name().to_string()));
            }

            let data = self.get_json(&path, &params, &referer).await?;
            if let Some(msg) = data
                .get("message")
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty())
            {
                bail!("reddit error for {}: {msg}", target.display());
            }

            let children = data
                .pointer("/data/children")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let mut added = 0usize;
            for child in &children {
                if child.get("kind").and_then(|k| k.as_str()) != Some("t3") {
                    continue;
                }
                let Some(d) = child.get("data") else { continue };
                let id = get_str(d, "id").unwrap_or_default();
                if id.is_empty() || !seen.insert(id) {
                    continue;
                }
                items.push(d.clone());
                added += 1;
            }
            page += 1;
            println!("  page {page}: {} posts (this page: {added})", items.len());

            if let Some(c) = cap
                && items.len() >= c as usize
            {
                break;
            }
            after = data
                .pointer("/data/after")
                .and_then(|a| a.as_str())
                .filter(|a| !a.is_empty())
                .map(str::to_string);
            if after.is_none() || added == 0 {
                break;
            }
            if self.page_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.page_delay_ms)).await;
            }
        }

        if let Some(c) = cap {
            items.truncate(c as usize);
        }
        Ok(items)
    }
}
