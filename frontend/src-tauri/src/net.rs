//! HTTP clients for services on this machine or the local network (Ollama).
//!
//! reqwest follows the operating system's proxy settings (on macOS it reads the
//! system proxy configuration). With a corporate proxy or VPN configured, requests
//! to e.g. http://192.168.3.16:11434 are sent to the proxy, which usually cannot
//! reach the LAN — while `curl`, which ignores system proxies, works. Local and
//! private addresses must therefore be contacted directly.

use std::net::IpAddr;

/// True for loopback, private (RFC 1918 / unique-local), link-local and CGNAT
/// addresses, `localhost`, single-label host names and `.local` / `.lan` / `.home`
/// / `.internal` names.
pub fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                let [a, b, ..] = v4.octets();
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || (a == 100 && (64..=127).contains(&b)) // CGNAT / Tailscale
            }
            IpAddr::V6(v6) => {
                let first = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (first & 0xfe00) == 0xfc00 // unique local
                    || (first & 0xffc0) == 0xfe80 // link local
            }
        };
    }
    !host.contains('.')
        || [".local", ".lan", ".home", ".internal", ".localdomain"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// A client for `url`: direct (no proxy) when the host is local or private,
/// otherwise the default client that honors proxy settings.
pub fn client_for(url: &str) -> reqwest::Client {
    let local = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(is_local_host))
        .unwrap_or(false);
    if local {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    } else {
        reqwest::Client::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_local_and_public_hosts() {
        for local in [
            "localhost",
            "127.0.0.1",
            "192.168.3.16",
            "10.0.0.5",
            "172.20.1.1",
            "169.254.1.1",
            "100.101.1.2",
            "::1",
            "[fd00::1]",
            "fe80::1",
            "ollama",
            "gpu-box.local",
            "server.lan",
        ] {
            assert!(is_local_host(local), "{local} should be local");
        }
        for public in ["8.8.8.8", "172.32.0.1", "api.openai.com", "ollama.example.com", "2001:4860::8888"] {
            assert!(!is_local_host(public), "{public} should not be local");
        }
    }

    #[test]
    fn client_for_accepts_any_url() {
        // Builds without panicking for local, public and invalid URLs
        let _ = client_for("http://192.168.3.16:11434");
        let _ = client_for("https://api.anthropic.com");
        let _ = client_for("not a url");
    }
}
