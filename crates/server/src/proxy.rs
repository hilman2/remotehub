//! The client's address behind reverse proxies (`REMOTEHUB_TRUSTED_PROXIES`).
//!
//! Behind a proxy, the connection comes from the proxy, and the client is
//! named in `X-Forwarded-For`, which every proxy on the way extends on the
//! right. Anyone can send that header, so it only counts when the connection
//! comes from a trusted proxy, and only as far as the chain of trusted
//! proxies reaches: the right-most address that is not a trusted proxy is the
//! client. Everything left of it was written by the client itself.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

/// An address range such as `10.0.0.0/8` or `fd00::/8`, or a single address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    address: IpAddr,
    prefix: u8,
}

impl Network {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl FromStr for Network {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address = address.parse::<IpAddr>().map_err(|_| ())?.to_canonical();
        let bits = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => bits,
            Some(prefix) => prefix.parse::<u8>().map_err(|_| ())?,
        };
        if prefix > bits {
            return Err(());
        }
        Ok(Network { address, prefix })
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

/// The client's address for a connection from `peer` that carries the
/// `X-Forwarded-For` header values `forwarded` (in the order received).
/// Without trusted proxies, or from a peer that is not one, it is `peer`.
pub fn client_address<'a>(
    peer: IpAddr,
    forwarded: impl DoubleEndedIterator<Item = &'a str>,
    trusted: &[Network],
) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|network| network.contains(ip));
    let mut client = peer.to_canonical();
    if !is_trusted(client) {
        return client;
    }
    let entries = forwarded.rev().flat_map(|value| value.rsplit(','));
    for entry in entries {
        // An entry that is no address ends the chain: whoever wrote it cannot
        // be trusted with the ones left of it either.
        let Some(ip) = parse(entry.trim()) else {
            break;
        };
        client = ip;
        if !is_trusted(ip) {
            break;
        }
    }
    client
}

/// An address as proxies write it: `1.2.3.4`, `2001:db8::1`, or with a port.
fn parse(entry: &str) -> Option<IpAddr> {
    entry
        .parse::<IpAddr>()
        .or_else(|_| entry.parse::<SocketAddr>().map(|address| address.ip()))
        .ok()
        .map(|ip| ip.to_canonical())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn networks(list: &[&str]) -> Vec<Network> {
        list.iter().map(|n| n.parse().unwrap()).collect()
    }

    fn client(peer: &str, forwarded: &[&str], trusted: &[&str]) -> String {
        client_address(
            peer.parse().unwrap(),
            forwarded.iter().copied(),
            &networks(trusted),
        )
        .to_string()
    }

    #[test]
    fn ranges_contain_what_their_prefix_covers() {
        let net: Network = "10.1.0.0/16".parse().unwrap();
        assert!(net.contains("10.1.255.255".parse().unwrap()));
        assert!(!net.contains("10.2.0.0".parse().unwrap()));
        assert!(net.contains("::ffff:10.1.0.1".parse().unwrap()));
        let one: Network = "192.0.2.1".parse().unwrap();
        assert!(one.contains("192.0.2.1".parse().unwrap()));
        assert!(!one.contains("192.0.2.2".parse().unwrap()));
        let all: Network = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains("203.0.113.9".parse().unwrap()));
        assert!(!all.contains("2001:db8::1".parse().unwrap()));
        let v6: Network = "fd00::/8".parse().unwrap();
        assert!(v6.contains("fd12::1".parse().unwrap()));
        assert!(!v6.contains("fe80::1".parse().unwrap()));
        for bad in [
            "10.0.0.0/33",
            "fd00::/129",
            "10.0.0/8",
            "proxy",
            "10.0.0.0/x",
            "",
        ] {
            assert!(bad.parse::<Network>().is_err(), "{bad}");
        }
    }

    #[test]
    fn only_a_trusted_peer_may_name_the_client() {
        let proxy = ["172.30.0.1"];
        assert_eq!(
            client("172.30.0.1", &["203.0.113.7"], &proxy),
            "203.0.113.7"
        );
        // Anyone else who sends the header is taken as who they are.
        assert_eq!(
            client("198.51.100.4", &["203.0.113.7"], &proxy),
            "198.51.100.4"
        );
        assert_eq!(client("172.30.0.1", &["203.0.113.7"], &[]), "172.30.0.1");
        // A trusted proxy that forwards nothing is the client itself.
        assert_eq!(client("172.30.0.1", &[], &proxy), "172.30.0.1");
    }

    #[test]
    fn what_the_client_wrote_itself_does_not_count() {
        let proxies = ["172.30.0.1", "10.0.0.0/8"];
        // The client sent "X-Forwarded-For: 192.0.2.66"; the proxy appended
        // the address it saw.
        assert_eq!(
            client("172.30.0.1", &["192.0.2.66, 203.0.113.7"], &proxies),
            "203.0.113.7"
        );
        // Two proxies, the outer one in 10/8; the headers may come split.
        assert_eq!(
            client(
                "172.30.0.1",
                &["192.0.2.66", "203.0.113.7, 10.0.0.5"],
                &proxies
            ),
            "203.0.113.7"
        );
        // Only proxies in the chain: the left-most is as far as it goes.
        assert_eq!(
            client("172.30.0.1", &["10.0.0.9, 10.0.0.5"], &proxies),
            "10.0.0.9"
        );
        // Garbage ends the chain at the last address before it.
        assert_eq!(
            client("172.30.0.1", &["203.0.113.7, unknown"], &proxies),
            "172.30.0.1"
        );
        assert_eq!(client("172.30.0.1", &["x, 10.0.0.5"], &proxies), "10.0.0.5");
    }

    #[test]
    fn addresses_come_in_every_form_proxies_write() {
        let proxy = ["::1"];
        assert_eq!(
            client("::1", &["[2001:db8::7]:4711"], &proxy),
            "2001:db8::7"
        );
        assert_eq!(client("::1", &["203.0.113.7:4711"], &proxy), "203.0.113.7");
        assert_eq!(
            client("::1", &["::ffff:203.0.113.7"], &proxy),
            "203.0.113.7"
        );
        // A peer on an IPv6 socket with a mapped IPv4 address.
        assert_eq!(client("::ffff:198.51.100.4", &[], &["::1"]), "198.51.100.4");
    }
}
