// SPDX-License-Identifier: AGPL-3.0-only
//! Outbound URL policy. Every URL a user can type (Git host, webhook, S3 endpoint) passes through
//! here before the control plane connects to it, so it cannot be aimed at the host's own network,
//! the cloud metadata service or the WireGuard overlay.
//!
//! Callers must connect to an address returned by [`resolve`] and not resolve the name again,
//! otherwise DNS rebinding could swap in a forbidden address after the check. They must also
//! disable redirect following and must not echo response bodies into error messages.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use url::{Host, Url};

/// The overlay network between cluster nodes. Never reachable from user-supplied URLs.
pub const WIREGUARD_RANGE: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 77, 0, 0), 16);

#[derive(Debug, Clone, Copy, Default)]
pub struct Policy {
    /// Lets an administrator allow private LAN addresses (RFC 1918, unique-local, CGNAT), for
    /// example a Gitea instance on the same network. Loopback, link-local, the WireGuard
    /// overlay and other special ranges stay blocked.
    pub allow_private: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Violation {
    #[error("only https URLs are allowed")]
    NotHttps,
    #[error("the URL is not valid")]
    Malformed,
    #[error("the URL must not contain a username or password")]
    Credentials,
    #[error("the host resolves to an address that is not allowed")]
    ForbiddenAddress,
    #[error("the host name could not be resolved")]
    Unresolvable,
}

fn in_v4(ip: Ipv4Addr, net: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
    u32::from(ip) & mask == u32::from(net) & mask
}

fn v4_blocked(ip: Ipv4Addr, policy: &Policy) -> bool {
    let (wg, wg_prefix) = WIREGUARD_RANGE;
    if ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() || ip.is_broadcast() || ip.is_multicast() || ip.is_documentation() {
        return true;
    }
    if in_v4(ip, wg, wg_prefix) || in_v4(ip, Ipv4Addr::new(0, 0, 0, 0), 8) || in_v4(ip, Ipv4Addr::new(240, 0, 0, 0), 4) {
        return true;
    }
    let private = ip.is_private() || in_v4(ip, Ipv4Addr::new(100, 64, 0, 0), 10);
    private && !policy.allow_private
}

fn v6_blocked(ip: Ipv6Addr, policy: &Policy) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_blocked(v4, policy);
    }
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let first = ip.segments()[0];
    let link_local = first & 0xffc0 == 0xfe80;
    let unique_local = first & 0xfe00 == 0xfc00;
    link_local || (unique_local && !policy.allow_private)
}

/// True when the policy forbids connecting to this address.
pub fn is_blocked(ip: IpAddr, policy: &Policy) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_blocked(v4, policy),
        IpAddr::V6(v6) => v6_blocked(v6, policy),
    }
}

/// Checks everything that can be decided without DNS: scheme, credentials and IP literals.
pub fn check_url(raw: &str, policy: &Policy) -> Result<Url, Violation> {
    let url = Url::parse(raw.trim()).map_err(|_| Violation::Malformed)?;
    if url.scheme() != "https" {
        return Err(Violation::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Violation::Credentials);
    }
    match url.host().ok_or(Violation::Malformed)? {
        Host::Ipv4(ip) if is_blocked(IpAddr::V4(ip), policy) => Err(Violation::ForbiddenAddress),
        Host::Ipv6(ip) if is_blocked(IpAddr::V6(ip), policy) => Err(Violation::ForbiddenAddress),
        Host::Domain(d) if d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost") => {
            Err(Violation::ForbiddenAddress)
        }
        _ => Ok(url),
    }
}

/// Resolves the host and rejects the URL if any resulting address is forbidden. Returns the
/// addresses to connect to.
pub async fn resolve(url: &Url, policy: &Policy) -> Result<Vec<SocketAddr>, Violation> {
    let port = url.port_or_known_default().ok_or(Violation::Malformed)?;
    let addrs: Vec<SocketAddr> = match url.host().ok_or(Violation::Malformed)? {
        Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        Host::Domain(d) => tokio::net::lookup_host((d, port)).await.map_err(|_| Violation::Unresolvable)?.collect(),
    };
    if addrs.is_empty() {
        return Err(Violation::Unresolvable);
    }
    // One forbidden answer rejects the whole name: a mixed answer is how rebinding attacks start.
    if addrs.iter().any(|a| is_blocked(a.ip(), policy)) {
        return Err(Violation::ForbiddenAddress);
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRICT: Policy = Policy { allow_private: false };
    const LAN: Policy = Policy { allow_private: true };

    #[test]
    fn rejects_the_addresses_the_success_criteria_name() {
        for url in ["http://169.254.169.254", "https://169.254.169.254", "https://10.77.0.1", "https://127.0.0.1"] {
            assert!(check_url(url, &STRICT).is_err(), "{url}");
            assert!(check_url(url, &LAN).is_err(), "{url} must stay blocked even when private is allowed");
        }
    }

    #[test]
    fn requires_https_and_forbids_credentials() {
        assert_eq!(check_url("http://example.com", &STRICT).unwrap_err(), Violation::NotHttps);
        assert_eq!(check_url("https://user:pw@example.com", &STRICT).unwrap_err(), Violation::Credentials);
        assert_eq!(check_url("not a url", &STRICT).unwrap_err(), Violation::Malformed);
        assert!(check_url("https://example.com/path?x=1", &STRICT).is_ok());
    }

    #[test]
    fn private_ranges_need_explicit_permission() {
        for ip in ["10.1.2.3", "172.16.0.1", "192.168.1.10", "100.64.0.1"] {
            let url = format!("https://{ip}");
            assert_eq!(check_url(&url, &STRICT).unwrap_err(), Violation::ForbiddenAddress, "{ip}");
            assert!(check_url(&url, &LAN).is_ok(), "{ip}");
        }
    }

    #[test]
    fn blocks_ipv6_and_mapped_ipv4_forms() {
        for url in [
            "https://[::1]",
            "https://[fe80::1]",
            "https://[::ffff:127.0.0.1]",
            "https://[::ffff:169.254.169.254]",
            "https://[fd00::1]",
            "https://[::]",
        ] {
            assert!(check_url(url, &STRICT).is_err(), "{url}");
        }
        assert!(check_url("https://[fd00::1]", &LAN).is_ok());
        assert!(check_url("https://[fe80::1]", &LAN).is_err());
        assert!(check_url("https://[2606:4700:4700::1111]", &STRICT).is_ok());
    }

    #[test]
    fn blocks_localhost_names_and_special_ranges() {
        for url in ["https://localhost", "https://app.localhost", "https://0.0.0.0", "https://224.0.0.1", "https://255.255.255.255"] {
            assert!(check_url(url, &LAN).is_err(), "{url}");
        }
    }

    #[tokio::test]
    async fn resolve_rejects_forbidden_literals() {
        let literal = Url::parse("https://127.0.0.1:8443").unwrap();
        assert_eq!(resolve(&literal, &STRICT).await.unwrap_err(), Violation::ForbiddenAddress);
        let public = Url::parse("https://93.184.216.34").unwrap();
        assert_eq!(resolve(&public, &STRICT).await.unwrap(), vec!["93.184.216.34:443".parse().unwrap()]);
    }
}
