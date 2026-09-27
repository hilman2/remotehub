//! Which web pages a vault login belongs to (#201, ADR 0017).
//!
//! The browser extension offers a login only on a page it belongs to, and
//! remotehub checks again before it hands the password out for filling. Both
//! apply the same rule; the extension's copy is `web/src/extension/site.ts`,
//! and the cases in `web/src/extension/site-cases.json` run against both.
//!
//! - Scheme and port must be the same. A login URL without a scheme counts
//!   as `https`, since people type `portal.example.com` into KeePass.
//! - Hosts are compared as the browser resolves them: lower case, with
//!   international names in punycode, without a trailing dot.
//! - Over `https`, a host matches every host with the same registrable
//!   domain by the Public Suffix List: `portal.example.com` matches
//!   `login.example.com`, but not `example.com.evil.net`, and
//!   `mine.github.io` does not match `other.github.io`.
//! - Only the very same host matches over `http`, and for an IP address, a
//!   host without a dot or a host that is a public suffix itself. Without
//!   TLS nothing vouches for a neighbour on the same domain.

use url::{Host, Url};

/// Whether the page at `page_origin` (`https://login.example.com`, as the
/// browser reports it) belongs to the login with the URL `login_url`.
pub fn matches(login_url: &str, page_origin: &str) -> bool {
    let (Some(login), Some(page)) = (parse(login_url), parse(page_origin)) else {
        return false;
    };
    if login.scheme() != page.scheme()
        || login.port_or_known_default() != page.port_or_known_default()
    {
        return false;
    }
    match (login.host(), page.host()) {
        (Some(Host::Domain(login_host)), Some(Host::Domain(page_host))) => {
            let (login_host, page_host) = (bare(login_host), bare(page_host));
            if login_host == page_host {
                return true;
            }
            if login.scheme() != "https" {
                return false;
            }
            match (registrable(login_host), registrable(page_host)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        }
        (Some(login_host), Some(page_host)) => login_host == page_host,
        _ => false,
    }
}

/// An `http` or `https` URL; one without a scheme is taken as `https`.
fn parse(text: &str) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let url = if text.contains("://") {
        Url::parse(text).ok()?
    } else {
        Url::parse(&format!("https://{text}")).ok()?
    };
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn bare(host: &str) -> &str {
    host.strip_suffix('.').unwrap_or(host)
}

/// The registrable domain of `host`; none for a host without a dot and for
/// a public suffix itself, which only match themselves.
fn registrable(host: &str) -> Option<&str> {
    if !host.contains('.') {
        return None;
    }
    psl::domain_str(host)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::matches;

    #[derive(Deserialize)]
    struct Case {
        login: String,
        page: String,
        matches: bool,
        why: String,
    }

    #[test]
    fn the_cases_the_extension_shares() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../../web/src/extension/site-cases.json"))
                .unwrap();
        assert!(cases.len() > 20);
        let wrong: Vec<String> = cases
            .iter()
            .filter(|case| matches(&case.login, &case.page) != case.matches)
            .map(|case| format!("{} on {}: {}", case.login, case.page, case.why))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }
}
