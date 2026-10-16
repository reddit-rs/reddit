//! Reddit session cookies from a Netscape `cookies.txt` export or an inline
//! `k=v; k2=v2` string.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;

/// Session cookies sent to Reddit. Values are validated when parsed, so the
/// resulting `Cookie` header is always well-formed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cookies(BTreeMap<String, String>);

impl Cookies {
    /// Parse `--cookies`: an existing file is read as a Netscape export;
    /// anything containing `=` is an inline `k=v; k2=v2` string.
    pub fn from_arg(arg: &str) -> Result<Self> {
        let path = Path::new(arg);
        if path.is_file() {
            Self::from_file(path)
        } else if arg.contains('=') {
            Self::from_header(arg)
        } else {
            bail!("cookies file not found: {arg}")
        }
    }

    /// Read a Netscape cookie export. Only unexpired cookies for `reddit.com`
    /// and its subdomains are kept; `#HttpOnly_` lines (used for session
    /// cookies such as `reddit_session`) are regular entries, not comments.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read cookies file {}", path.display()))?;
        let cookies = Self::from_netscape(&text, chrono::Utc::now().timestamp());
        if cookies.is_empty() {
            bail!(
                "no unexpired reddit.com cookies in {}; export them again from a signed-in browser",
                path.display()
            );
        }
        Ok(cookies)
    }

    /// Parse Netscape export text; cookies that expired before `now` (Unix
    /// seconds) are skipped. An expiry of `0` marks a session cookie. Entries
    /// that cannot be sent in a header are skipped with a warning.
    pub fn from_netscape(text: &str, now: i64) -> Self {
        let mut cookies = Self::default();
        for line in text.lines() {
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line).trim_end();
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let [domain, _, _, _, expires, name, value] =
                line.splitn(7, '\t').collect::<Vec<_>>()[..]
            else {
                continue;
            };
            let domain = domain.trim_start_matches('.').to_ascii_lowercase();
            let expired = expires.parse::<i64>().is_ok_and(|t| t != 0 && t < now);
            if (domain == "reddit.com" || domain.ends_with(".reddit.com"))
                && !expired
                && let Err(e) = cookies.insert(name, value)
            {
                eprintln!("warning: skipping cookie: {e}");
            }
        }
        cookies
    }

    /// Parse an inline `k=v; k2=v2` string.
    pub fn from_header(header: &str) -> Result<Self> {
        let mut cookies = Self::default();
        for pair in header.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .with_context(|| format!("invalid cookie '{pair}': expected name=value"))?;
            cookies.insert(name.trim(), value.trim())?;
        }
        Ok(cookies)
    }

    /// Reject anything that would corrupt the `Cookie` header: names must be
    /// visible ASCII without `=`/`;`, values visible ASCII without `;`.
    fn insert(&mut self, name: &str, value: &str) -> Result<()> {
        let name_ok = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'=' && b != b';');
        let value_ok = value.bytes().all(|b| b.is_ascii_graphic() && b != b';');
        if !name_ok || !value_ok {
            bail!("invalid characters in cookie '{}'", name.escape_debug());
        }
        self.0.insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    /// Add a cookie unless one with the same name exists.
    pub(crate) fn insert_default(&mut self, name: &str, value: &str) {
        self.0
            .entry(name.to_owned())
            .or_insert_with(|| value.to_owned());
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// `Cookie` header value; `None` when empty.
    pub fn header(&self) -> Option<String> {
        (!self.is_empty()).then(|| {
            self.0
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    /// The `Cookie` header marked sensitive; `None` when empty.
    pub(crate) fn header_value(&self) -> Option<wreq::header::HeaderValue> {
        self.header().map(|cookie| {
            // `insert` only admits visible ASCII, which is always header-safe.
            let mut value =
                wreq::header::HeaderValue::from_str(&cookie).expect("cookies are header-safe");
            value.set_sensitive(true);
            value
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_cookie_strings() {
        let cookies = Cookies::from_header("a=1; b=two;  x = y ;").unwrap();
        assert_eq!(cookies.len(), 3);
        assert_eq!(cookies.get("b"), Some("two"));
        assert_eq!(cookies.get("x"), Some("y"));
        assert_eq!(cookies.header().as_deref(), Some("a=1; b=two; x=y"));
        assert!(Cookies::from_header("novalue").is_err());
        assert!(Cookies::from_header("a=line\nbreak").is_err());
        assert!(Cookies::default().header().is_none());
    }

    #[test]
    fn netscape_exports_keep_unexpired_reddit_cookies() {
        let text = "# Netscape HTTP Cookie File\n\
             #HttpOnly_.reddit.com\tTRUE\t/\tTRUE\t0\treddit_session\tSID\n\
             .reddit.com\tTRUE\t/\tFALSE\t2000\tcsrf_token\tTOK\n\
             www.reddit.com\tFALSE\t/\tFALSE\t999\texpired\tOLD\n\
             .example.org\tTRUE\t/\tTRUE\t0\tunrelated_session\tPRIVATE\n\
             .reddit.com.example.org\tTRUE\t/\tTRUE\t0\tcsrf_token\tWRONG\n\
             .reddit.com\tTRUE\t/\tTRUE\t0\tbad\tline;break\n\
             malformed line\n";
        let cookies = Cookies::from_netscape(text, 1000);
        assert_eq!(cookies.get("reddit_session"), Some("SID"));
        assert_eq!(cookies.get("csrf_token"), Some("TOK"));
        assert_eq!(cookies.len(), 2);
    }

    #[test]
    fn arguments_distinguish_files_from_inline_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cookies.txt");
        std::fs::write(&file, ".reddit.com\tTRUE\t/\tTRUE\t0\ttoken_v2\tT\n").unwrap();
        let cookies = Cookies::from_arg(file.to_str().unwrap()).unwrap();
        assert_eq!(cookies.get("token_v2"), Some("T"));

        assert_eq!(Cookies::from_arg("a=1").unwrap().len(), 1);
        let missing = Cookies::from_arg("missing-cookies.txt").unwrap_err();
        assert!(missing.to_string().contains("not found"), "{missing}");

        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, "# Netscape HTTP Cookie File\n").unwrap();
        assert!(Cookies::from_arg(empty.to_str().unwrap()).is_err());
    }

    #[test]
    fn defaults_do_not_override_existing_values() {
        let mut cookies = Cookies::from_header("over18=0").unwrap();
        cookies.insert_default("over18", "1");
        cookies.insert_default("lang", "en");
        assert_eq!(cookies.get("over18"), Some("0"));
        assert_eq!(cookies.get("lang"), Some("en"));
    }
}
