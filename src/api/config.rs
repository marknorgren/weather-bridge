//! Deployment settings for the HTTP app, parsed once by the entry point.
use super::OriginVerify;
use std::sync::Arc;

/// Everything `app` needs besides the weather service. `Default` is the local setup:
/// loopback-only MCP, no origin check and the published developer docs.
#[derive(Clone, Debug, Default)]
pub struct HttpConfig {
    pub mcp_access: McpAccess,
    pub origin_verify: Option<OriginVerify>,
    pub docs_url: DocsUrl,
}

/// Trusted MCP hosts and origins, parsed once by the entry point. `None` keeps the
/// loopback-only defaults.
#[derive(Clone, Debug, Default)]
pub struct McpAccess {
    pub hosts: Option<Vec<String>>,
    pub origins: Option<Vec<String>>,
}

/// Static developer documentation (REST reference, MCP guide) published on GitHub Pages,
/// built from `docs/site` by `scripts/build-docs.py`. Default for `--docs-url`.
pub const DEFAULT_DOCS_URL: &str = "https://marknorgren.github.io/weather-bridge/";

/// Base URL of the developer docs. `/developer` links to it and to its `rest.html`
/// through the `@@DOCS_URL@@` placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocsUrl(Arc<str>);
impl DocsUrl {
    /// Accept an absolute HTTPS URL without credentials, query or fragment, adding a
    /// trailing `/` when missing. Parsing percent-encodes quotes and angle brackets, so the
    /// result is safe inside an HTML attribute.
    pub fn new(value: &str) -> Result<Self, String> {
        let invalid =
            || "must be an https:// URL without credentials, query or fragment".to_owned();
        let mut url = reqwest::Url::parse(value).map_err(|_| invalid())?;
        if url.scheme() != "https"
            || url.host_str().is_none_or(str::is_empty)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self(url.as_str().into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl Default for DocsUrl {
    fn default() -> Self {
        Self(DEFAULT_DOCS_URL.into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn docs_url_requires_https_and_normalizes_the_trailing_slash() {
        assert_eq!(DocsUrl::default().as_str(), DEFAULT_DOCS_URL);
        assert_eq!(
            DocsUrl::new(DEFAULT_DOCS_URL).unwrap().as_str(),
            DEFAULT_DOCS_URL
        );
        // A fork's Pages URL without a trailing slash still links to rest.html beneath it.
        assert_eq!(
            DocsUrl::new("https://docs.example.com/fork")
                .unwrap()
                .as_str(),
            "https://docs.example.com/fork/"
        );
        for bad in [
            "",
            "docs.example.com/",
            "javascript:alert(1)",
            "http://docs.example.com/",
            "https://user:pass@docs.example.com/",
            "https://docs.example.com/?token=x",
            "https://docs.example.com/#fragment",
        ] {
            assert!(DocsUrl::new(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn docs_url_is_safe_inside_an_html_attribute() {
        // URL parsing percent-encodes quotes and angle brackets, so the value cannot end
        // the href attribute or open a tag.
        let url = DocsUrl::new("https://docs.example.com/a\"onmouseover=x<b>/").unwrap();
        assert!(
            !url.as_str().contains(['"', '<', '>', '\'']),
            "{}",
            url.as_str()
        );
    }
}
