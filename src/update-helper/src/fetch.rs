//! Fetch abstraction: the seam between the real network (GitHub) and
//! offline fixtures.
//!
//! Production URLs are `https://`; test fixtures and offline runs use
//! `file://` URLs, which address plain files on disk. The core logic
//! (discovery, authentication, pin construction) never learns which
//! kind it is talking to.

use anyhow::Context;

/// A fetched document, plus the `Link: rel="next"` pagination pointer
/// (HTTP only; `file://` fetches are single-page).
#[derive(Debug)]
pub struct Response {
    pub body: Vec<u8>,
    pub next: Option<String>,
}

pub trait Fetch: Send {
    fn get(&self, url: &str) -> anyhow::Result<Response>;
}

/// Reads `file://` URLs from the local filesystem.
pub struct FileFetch;

impl Fetch for FileFetch {
    fn get(&self, url: &str) -> anyhow::Result<Response> {
        let path = url
            .strip_prefix("file://")
            .ok_or_else(|| anyhow::anyhow!("not a file:// URL: {url}"))?;
        let body = std::fs::read(path).with_context(|| format!("cannot read {path}"))?;
        Ok(Response { body, next: None })
    }
}

/// Plain HTTPS over ureq (rustls; the image carries no OpenSSL dev
/// packages). Follows GitHub's `Link` pagination header.
pub struct HttpFetch;

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> anyhow::Result<Response> {
        let res = ureq::get(url).call().with_context(|| format!("HTTP request to {url}"))?;
        let next = res
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_link_next);
        let body = res
            .into_body()
            .read_to_vec()
            .with_context(|| format!("reading body of {url}"))?;
        Ok(Response { body, next })
    }
}

/// Chooses the fetcher for a URL by scheme.
pub fn fetcher_for(url: &str) -> Box<dyn Fetch> {
    match scheme(url) {
        "file" => Box::new(FileFetch),
        "http" | "https" => Box::new(HttpFetch),
        _ => Box::new(NoFetch),
    }
}

/// A fetcher that always fails: non-URL inputs (missing scheme).
struct NoFetch;

impl Fetch for NoFetch {
    fn get(&self, url: &str) -> anyhow::Result<Response> {
        anyhow::bail!("unsupported URL scheme: {url} (expected http, https, or file)")
    }
}

fn scheme(url: &str) -> &str {
    url.split("://").next().unwrap_or("")
}

/// Parses the RFC 5988 `Link` header and returns the `rel="next"`
/// target, if any. GitHub paginates with
/// `Link: <https://…>; rel="next", <https://…>; rel="last"`.
pub fn parse_link_next(header: &str) -> Option<String> {
    for part in header.split(',') {
        let mut link = None;
        let mut rel = None;
        for seg in part.split(';') {
            let seg = seg.trim();
            if let Some(u) = seg.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
                link = Some(u.to_string());
            } else if let Some(r) = seg.strip_prefix("rel=") {
                rel = Some(r.trim_matches('"').to_string());
            }
        }
        if rel.as_deref() == Some("next") {
            return link;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_link_next;

    #[test]
    fn parses_github_link_pagination() {
        let header = "<https://api.github.com/repos/o/r/releases?page=2>; rel=\"next\", \
                      <https://api.github.com/repos/o/r/releases?page=5>; rel=\"last\"";
        assert_eq!(
            parse_link_next(header).as_deref(),
            Some("https://api.github.com/repos/o/r/releases?page=2")
        );
    }

    #[test]
    fn no_next_yields_none() {
        assert_eq!(
            parse_link_next("<https://api.github.com/x?page=3>; rel=\"last\""),
            None
        );
        assert_eq!(parse_link_next(""), None);
    }
}
