//! Shared HTTP client and retry policy.

use anyhow::{Context, Result};
use std::time::Duration;
use wreq::Response;
use wreq::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use wreq_util::Profile;

/// Browser profile for TLS/HTTP2 fingerprints, client hints and the default
/// User-Agent. Reddit rejects clients that do not look like a browser.
const PROFILE: Profile = Profile::Chrome149;

/// Attempts per URL before giving up.
pub(crate) const ATTEMPTS: u32 = 4;

/// Longest server-requested wait that is honored.
const MAX_WAIT: Duration = Duration::from_secs(300);

/// Build the client shared by API and media requests. `user_agent` replaces
/// the profile's own (Chrome 149 on macOS); it should match the browser the
/// cookies came from.
pub(crate) fn client(user_agent: Option<&str>) -> Result<wreq::Client> {
    let mut builder = wreq::Client::builder()
        .emulation(PROFILE)
        .connect_timeout(Duration::from_secs(20));
    if let Some(ua) = user_agent {
        builder = builder.user_agent(HeaderValue::from_str(ua).context("invalid --user-agent")?);
    }
    builder.build().context("cannot build HTTP client")
}

/// Exponential backoff for transient failures: 1s, 2s, 4s, …
pub(crate) fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1 << attempt.min(6))
}

/// Wait requested by a rate-limited response (`Retry-After` or Reddit's
/// `x-ratelimit-reset`, in seconds), else a growing default.
pub(crate) fn rate_limit_wait(headers: &HeaderMap, attempt: u32) -> Duration {
    seconds_header(headers, RETRY_AFTER.as_str())
        .or_else(|| seconds_header(headers, "x-ratelimit-reset"))
        .unwrap_or(Duration::from_secs(15 * u64::from(attempt + 1)))
        .min(MAX_WAIT)
}

/// When Reddit reports an exhausted request budget, the time until it resets.
pub(crate) fn budget_exhausted(response: &Response) -> Option<Duration> {
    let headers = response.headers();
    let remaining: f64 = headers
        .get("x-ratelimit-remaining")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (remaining < 1.0)
        .then(|| seconds_header(headers, "x-ratelimit-reset"))
        .flatten()
        .map(|wait| wait.min(MAX_WAIT))
}

fn seconds_header(headers: &HeaderMap, name: &str) -> Option<Duration> {
    let seconds: f64 = headers.get(name)?.to_str().ok()?.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds.ceil()))
}

/// Session cookies are only sent to Reddit itself over HTTPS, never to
/// external link targets or media CDNs.
pub(crate) fn is_secure_reddit_url(url: &str) -> bool {
    url.parse::<wreq::Uri>().is_ok_and(|uri| {
        uri.scheme_str() == Some("https")
            && uri.host().is_some_and(|host| {
                let host = host.to_ascii_lowercase();
                host == "reddit.com" || host.ends_with(".reddit.com")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_headers_decide_the_wait() {
        let mut headers = HeaderMap::new();
        assert_eq!(rate_limit_wait(&headers, 1), Duration::from_secs(30));
        headers.insert("x-ratelimit-reset", HeaderValue::from_static("42.3"));
        assert_eq!(rate_limit_wait(&headers, 0), Duration::from_secs(43));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(rate_limit_wait(&headers, 0), Duration::from_secs(7));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("99999"));
        assert_eq!(rate_limit_wait(&headers, 0), MAX_WAIT);
    }

    #[test]
    fn backoff_grows_and_is_bounded() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(60), Duration::from_secs(64));
    }

    #[test]
    fn cookies_are_scoped_to_secure_reddit_hosts() {
        for url in [
            "https://reddit.com/file",
            "https://www.reddit.com/file",
            "https://OLD.reddit.com/",
        ] {
            assert!(is_secure_reddit_url(url), "{url}");
        }
        for url in [
            "http://www.reddit.com/file",
            "https://i.redd.it/file",
            "https://reddit.com.example.org/file",
            "https://example.org/file",
            "https://reddit.com@example.org/file",
            "invalid",
        ] {
            assert!(!is_secure_reddit_url(url), "{url}");
        }
    }

    #[test]
    fn default_and_custom_user_agents_build() {
        assert!(client(None).is_ok());
        assert!(client(Some("Mozilla/5.0 test")).is_ok());
        assert!(client(Some("bad\nagent")).is_err());
    }
}
