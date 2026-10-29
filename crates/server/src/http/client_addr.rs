use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

use axum::http::{HeaderMap, HeaderName};

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// An IP network such as `172.18.0.0/16`; a bare address is a network of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    #[must_use]
    pub fn contains(&self, addr: IpAddr) -> bool {
        let addr = addr.to_canonical();
        addr.is_ipv4() == self.network.is_ipv4() && mask(addr, self.prefix) == self.network
    }
}

impl FromStr for Cidr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (s, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("`{s}` is not an IP address or network"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(prefix) => prefix
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("`{s}`: the prefix length must be 0 to {max}"))?,
        };
        Ok(Self {
            network: mask(addr, prefix),
            prefix,
        })
    }
}

fn mask(addr: IpAddr, prefix: u8) -> IpAddr {
    match addr {
        IpAddr::V4(v4) => {
            let bits = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V4(Ipv4Addr::from(u32::from(v4) & bits))
        }
        IpAddr::V6(v6) => {
            let bits = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & bits))
        }
    }
}

/// The address a request comes from: the TCP peer, or, when the peer is a trusted proxy, the
/// rightmost `X-Forwarded-For` hop that is not itself one. Hops left of an untrusted one are
/// set by the client and ignored; an unreadable hop stops the walk at the proxy that sent it.
#[must_use]
pub fn client_addr(peer: IpAddr, headers: &HeaderMap, trusted: &[Cidr]) -> IpAddr {
    let is_trusted = |addr: IpAddr| trusted.iter().any(|cidr| cidr.contains(addr));
    let mut client = peer.to_canonical();
    if !is_trusted(client) {
        return client;
    }
    for value in headers.get_all(X_FORWARDED_FOR).iter().rev() {
        for hop in value.as_bytes().rsplit(|b| *b == b',') {
            let Some(addr) = parse_hop(hop) else {
                return client;
            };
            client = addr;
            if !is_trusted(addr) {
                return client;
            }
        }
    }
    client
}

fn parse_hop(hop: &[u8]) -> Option<IpAddr> {
    let hop = std::str::from_utf8(hop).ok()?.trim();
    hop.parse::<IpAddr>()
        .or_else(|_| hop.parse::<SocketAddr>().map(|addr| addr.ip()))
        .ok()
        .map(|addr| addr.to_canonical())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn cidrs(list: &[&str]) -> Vec<Cidr> {
        list.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn forwarded(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(X_FORWARDED_FOR, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn parses_networks_and_single_addresses() {
        let net: Cidr = "172.18.3.4/16".parse().unwrap();
        assert!(net.contains(ip("172.18.200.1")));
        assert!(!net.contains(ip("172.19.0.1")));
        assert!(net.contains(ip("::ffff:172.18.0.1")));
        let one: Cidr = "10.0.0.1".parse().unwrap();
        assert!(one.contains(ip("10.0.0.1")));
        assert!(!one.contains(ip("10.0.0.2")));
        let v6: Cidr = "fd00::/8".parse().unwrap();
        assert!(v6.contains(ip("fd12::1")));
        assert!(!v6.contains(ip("fe80::1")));
        let all: Cidr = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains(ip("8.8.8.8")));
        assert!(!all.contains(ip("::1")));
    }

    #[test]
    fn rejects_malformed_networks() {
        for bad in [
            "",
            "10.0.0.0/33",
            "::/129",
            "10.0.0.0/",
            "10.0.0/8",
            "host/8",
            "1.2.3.4/-1",
        ] {
            assert!(bad.parse::<Cidr>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn untrusted_peer_is_the_client_whatever_it_forwards() {
        let headers = forwarded(&["1.1.1.1"]);
        assert_eq!(
            client_addr(ip("203.0.113.9"), &headers, &[]),
            ip("203.0.113.9")
        );
        let trusted = cidrs(&["172.18.0.0/16"]);
        assert_eq!(
            client_addr(ip("203.0.113.9"), &headers, &trusted),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn trusted_peer_yields_the_rightmost_untrusted_hop() {
        let trusted = cidrs(&["172.18.0.0/16", "10.0.0.1"]);
        let peer = ip("172.18.0.2");
        // The client prepended a forged hop; the proxies appended the real one.
        let headers = forwarded(&["6.6.6.6, 198.51.100.7", "10.0.0.1"]);
        assert_eq!(client_addr(peer, &headers, &trusted), ip("198.51.100.7"));
        let headers = forwarded(&["198.51.100.7:4321"]);
        assert_eq!(client_addr(peer, &headers, &trusted), ip("198.51.100.7"));
        let headers = forwarded(&["[2001:db8::1]:443"]);
        assert_eq!(client_addr(peer, &headers, &trusted), ip("2001:db8::1"));
    }

    #[test]
    fn trusted_peer_without_usable_hops_stays_the_last_trusted_one() {
        let trusted = cidrs(&["172.18.0.0/16", "10.0.0.1"]);
        let peer = ip("172.18.0.2");
        assert_eq!(client_addr(peer, &HeaderMap::new(), &trusted), peer);
        let headers = forwarded(&["10.0.0.1"]);
        assert_eq!(client_addr(peer, &headers, &trusted), ip("10.0.0.1"));
        let headers = forwarded(&["1.1.1.1, garbage, 10.0.0.1"]);
        assert_eq!(client_addr(peer, &headers, &trusted), ip("10.0.0.1"));
    }

    #[test]
    fn unreadable_bytes_left_of_the_appended_hop_do_not_hide_the_client() {
        let trusted = cidrs(&["172.18.0.0/16"]);
        let peer = ip("172.18.0.2");
        let mut headers = HeaderMap::new();
        headers.append(
            X_FORWARDED_FOR,
            HeaderValue::from_bytes(b"\xff\xfe garbage, 198.51.100.7").unwrap(),
        );
        assert_eq!(client_addr(peer, &headers, &trusted), ip("198.51.100.7"));
    }
}
