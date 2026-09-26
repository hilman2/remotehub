//! Address ranges such as `10.0.0.0/8`.

use std::fmt;
use std::net::IpAddr;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
