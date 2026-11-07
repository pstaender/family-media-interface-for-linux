//! Website rules: which pages a website tile may show.
//!
//! Shared by the media center (`src/`, config validation and building the allow list) and the
//! browser window (`web/`, which includes this file via `#[path]` and enforces the rules).
//! Only depends on the `url` crate, so it is tested without WebKitGTK.
//!
//! - **Pages** (the main frame): only the website's domain. With "allow all subdomains" that is
//!   the start host without a leading `www.` plus all its subdomains (`kika.de`, `*.kika.de`);
//!   otherwise exactly the start host.
//! - **Everything else** (pictures, videos, scripts, embedded players/frames): any address —
//!   websites often use external CDNs — except local addresses (`localhost`, `*.local`, private
//!   and loopback IPs) and non-web schemes (`file:` …), which are always blocked so a website
//!   can never reach the admin interface or other devices at home.
//!
//! Hosts are compared after URL normalization (WHATWG: lowercase, IDN → punycode), never as
//! substrings: `kika.de.evil.com`, `evil.com/?kika.de` and `kika.de@evil.com` don't match.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Exactly this host.
    Exact(String),
    /// This domain and all its subdomains (`*.domain`).
    WithSubdomains(String),
}

impl Rule {
    /// `kika.de` → exact, `*.kika.de` → with subdomains. `None` for anything that isn't a
    /// normalized, non-local domain name.
    pub fn parse(text: &str) -> Option<Rule> {
        let (wildcard, domain) = match text.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let url = Url::parse(&format!("https://{domain}/")).ok()?;
        let host = page_host(&url)?;
        if host != domain {
            return None; // not normalized (uppercase, IDN, trailing dot, …)
        }
        Some(if wildcard {
            Rule::WithSubdomains(host)
        } else {
            Rule::Exact(host)
        })
    }

    /// The form used on the command line of the browser window.
    pub fn to_arg(&self) -> String {
        match self {
            Rule::Exact(d) => d.clone(),
            Rule::WithSubdomains(d) => format!("*.{d}"),
        }
    }

    fn matches(&self, host: &str) -> bool {
        match self {
            Rule::Exact(d) => host == d,
            Rule::WithSubdomains(d) => {
                host == d
                    || host
                        .strip_suffix(d.as_str())
                        .is_some_and(|prefix| prefix.ends_with('.'))
            }
        }
    }
}

/// The normalized host of an http(s) URL, if it is a domain name a website may use as page
/// (not an IP address, not `localhost`/`*.localhost`/`*.local`, has a dot).
pub fn page_host(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    match url.host()? {
        Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            let local = d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local");
            (!d.is_empty() && d.contains('.') && !local).then_some(d)
        }
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

/// Allow rules for a website.
pub fn rules_for(start_url: &str, allow_subdomains: bool) -> Result<Vec<Rule>, String> {
    let url = Url::parse(start_url).map_err(|e| format!("invalid address: {e}"))?;
    let host = page_host(&url).ok_or("the address must be http(s):// with a domain name")?;
    Ok(if allow_subdomains {
        let base = host.strip_prefix("www.").unwrap_or(&host).to_string();
        vec![Rule::Exact(base.clone()), Rule::WithSubdomains(base)]
    } else {
        vec![Rule::Exact(host)]
    })
}

/// May the main frame show this page?
pub fn is_allowed_page(url: &str, rules: &[Rule]) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    page_host(&url).is_some_and(|host| rules.iter().any(|r| r.matches(&host)))
}

/// Requests that are blocked everywhere (any frame): local addresses and non-web schemes.
pub fn is_always_blocked(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return true;
    };
    match url.scheme() {
        "http" | "https" | "ws" | "wss" => match url.host() {
            None => true,
            Some(Host::Domain(d)) => {
                let d = d.trim_end_matches('.').to_ascii_lowercase();
                d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local")
            }
            Some(Host::Ipv4(ip)) => is_local_v4(ip),
            Some(Host::Ipv6(ip)) => ip.to_ipv4_mapped().is_some_and(is_local_v4) || is_local_v6(ip),
        },
        // In-page content without a network address.
        "about" | "data" | "blob" => false,
        // file:, ftp:, custom schemes, …
        _ => true,
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.octets()[0] == 0
        // 100.64.0.0/10 (carrier-grade NAT, also used by some home VPNs)
        || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64)
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || (first & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (first & 0xffc0) == 0xfe80 // link local fe80::/10
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(url: &str, sub: bool) -> Vec<Rule> {
        rules_for(url, sub).unwrap()
    }

    #[test]
    fn subdomain_rules_strip_www() {
        assert_eq!(
            rules("https://www.kika.de/start", true),
            [
                Rule::Exact("kika.de".into()),
                Rule::WithSubdomains("kika.de".into())
            ]
        );
        assert_eq!(
            rules("https://www.kika.de/", false),
            [Rule::Exact("www.kika.de".into())]
        );
    }

    #[test]
    fn pages_with_subdomains() {
        let r = rules("https://www.kika.de/", true);
        for ok in [
            "https://kika.de/",
            "https://www.kika.de/a?b=c#d",
            "http://media.kika.de/x",
            "https://WWW.KIKA.DE/",
            "https://kika.de./",
            "https://a.b.kika.de:8443/",
            "about:blank",
        ] {
            assert!(is_allowed_page(ok, &r), "{ok}");
        }
        for bad in [
            "https://evil.com/?kika.de",
            "https://kika.de.evil.com/",
            "https://notkika.de/",
            "https://evilkika.de/",
            "https://kika.de@evil.com/",
            "https://evil.com/kika.de",
            "ftp://kika.de/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://127.0.0.1/",
            "data:text/html,hi",
            "not a url",
        ] {
            assert!(!is_allowed_page(bad, &r), "{bad}");
        }
    }

    #[test]
    fn pages_without_subdomains() {
        let r = rules("https://www.kika.de/", false);
        assert!(is_allowed_page("https://www.kika.de/x", &r));
        assert!(!is_allowed_page("https://kika.de/", &r));
        assert!(!is_allowed_page("https://media.kika.de/", &r));
    }

    #[test]
    fn idn_hosts_are_compared_normalized() {
        let r = rules("https://www.müller.de/", true);
        assert_eq!(r[0], Rule::Exact("xn--mller-kva.de".into()));
        assert!(is_allowed_page("https://shop.MÜLLER.de/", &r));
        assert!(!is_allowed_page("https://muller.de/", &r));
    }

    #[test]
    fn start_addresses_must_be_public_domains() {
        for bad in [
            "http://localhost:8787/",
            "https://192.168.1.1/",
            "https://[::1]/",
            "https://printer.local/",
            "https://intranet/",
            "file:///home",
            "kika.de",
            "",
        ] {
            assert!(rules_for(bad, true).is_err(), "{bad}");
        }
    }

    #[test]
    fn local_addresses_are_always_blocked() {
        for blocked in [
            "http://localhost:8787/api/config",
            "http://127.0.0.1:8787/",
            "http://127.1/",
            "http://0x7f000001/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://192.168.178.1/",
            "http://10.0.0.5/",
            "http://172.16.0.1/",
            "http://169.254.1.1/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://router.local/",
            "http://app.localhost/",
            "http://0.0.0.0/",
            "file:///etc/passwd",
            "ftp://example.com/",
            "garbage",
        ] {
            assert!(is_always_blocked(blocked), "{blocked}");
        }
        for ok in [
            "https://cdn.example-cdn.net/video.mp4",
            "https://www.youtube-nocookie.com/embed/x",
            "https://8.8.8.8/",
            "wss://chat.example.com/",
            "about:blank",
            "data:image/png;base64,AAAA",
            "blob:https://kika.de/1234",
        ] {
            assert!(!is_always_blocked(ok), "{ok}");
        }
    }

    #[test]
    fn rule_args_roundtrip() {
        for arg in ["kika.de", "*.kika.de", "xn--mller-kva.de"] {
            assert_eq!(Rule::parse(arg).unwrap().to_arg(), arg);
        }
        for bad in [
            "KIKA.de",
            "*.",
            "localhost",
            "*.local",
            "127.0.0.1",
            "kika.de.",
            "a b",
        ] {
            assert_eq!(Rule::parse(bad), None, "{bad}");
        }
    }
}
