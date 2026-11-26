// SPDX-License-Identifier: AGPL-3.0-only
//! Works out the client address, believing `X-Forwarded-For` only from configured proxies.
use std::net::IpAddr;

use axum::http::HeaderMap;
use ipnet::IpNet;

pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted: &[IpNet]) -> Option<IpAddr> {
    let is_trusted = |ip: &IpAddr| trusted.iter().any(|net| net.contains(ip));
    let peer = peer?;
    if !is_trusted(&peer) {
        return Some(peer);
    }
    let forwarded = headers.get_all("x-forwarded-for").iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(','));
    // Walk from the proxy side inwards; the first address that is not one of ours is the client.
    let hops: Vec<IpAddr> = forwarded.filter_map(|s| s.trim().parse().ok()).collect();
    Some(hops.into_iter().rev().find(|ip| !is_trusted(ip)).unwrap_or(peer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(xff: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", xff.parse().unwrap());
        h
    }

    #[test]
    fn ignores_forwarded_header_from_untrusted_peers() {
        let ip = client_ip(&headers("1.2.3.4"), Some("203.0.113.9".parse().unwrap()), &[]);
        assert_eq!(ip, Some("203.0.113.9".parse().unwrap()));
    }

    #[test]
    fn uses_the_first_untrusted_hop_from_the_right() {
        let trusted: Vec<IpNet> = vec!["10.0.0.0/8".parse().unwrap()];
        // A client-supplied `9.9.9.9` is to the left of the address our proxy actually saw.
        let ip = client_ip(&headers("9.9.9.9, 198.51.100.7, 10.0.0.2"), Some("10.0.0.1".parse().unwrap()), &trusted);
        assert_eq!(ip, Some("198.51.100.7".parse().unwrap()));
    }
}
