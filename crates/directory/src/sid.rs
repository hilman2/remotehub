//! Security identifiers (SIDs) of Active Directory — the immutable IDs by
//! which remotehub stores users and groups (ADR 0005).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A SID in its string form `S-1-5-21-…`, validated on construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sid(String);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid SID")]
pub struct InvalidSid;

impl Sid {
    /// Parses the binary form of `objectSid` and `tokenGroups`: revision,
    /// sub-authority count, 48-bit authority (big-endian), then 32-bit
    /// sub-authorities (little-endian).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, InvalidSid> {
        let (&revision, rest) = bytes.split_first().ok_or(InvalidSid)?;
        let (&count, rest) = rest.split_first().ok_or(InvalidSid)?;
        if revision != 1 || rest.len() != 6 + 4 * usize::from(count) {
            return Err(InvalidSid);
        }
        let (authority, subs) = rest.split_at(6);
        let authority = authority
            .iter()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
        let mut sid = format!("S-{revision}-{authority}");
        for chunk in subs.as_chunks::<4>().0 {
            let sub = u32::from_le_bytes(*chunk);
            sid.push_str(&format!("-{sub}"));
        }
        Ok(Sid(sid))
    }

    /// The binary form, as `objectSid` holds it; the inverse of
    /// [`Sid::from_bytes`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut parts = self.0.split('-').skip(2);
        let authority: u64 = parts.next().and_then(|a| a.parse().ok()).unwrap_or(0);
        let subs: Vec<u32> = parts.filter_map(|s| s.parse().ok()).collect();
        let mut bytes = vec![1, subs.len() as u8];
        bytes.extend_from_slice(&authority.to_be_bytes()[2..]);
        for sub in subs {
            bytes.extend_from_slice(&sub.to_le_bytes());
        }
        bytes
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The relative identifier: the last sub-authority (e.g. 513 for Domain Users).
    pub fn rid(&self) -> Option<u32> {
        self.0.rsplit('-').next()?.parse().ok()
    }
}

impl FromStr for Sid {
    type Err = InvalidSid;

    fn from_str(s: &str) -> Result<Self, InvalidSid> {
        let mut parts = s.split('-');
        if parts.next() != Some("S") || parts.next() != Some("1") {
            return Err(InvalidSid);
        }
        let numbers: Vec<&str> = parts.collect();
        let valid = !numbers.is_empty()
            && numbers.len() <= 16
            && numbers[0].parse::<u64>().is_ok_and(|a| a < 1 << 48)
            && numbers[1..].iter().all(|n| n.parse::<u32>().is_ok());
        if valid {
            Ok(Sid(s.to_owned()))
        } else {
            Err(InvalidSid)
        }
    }
}

impl TryFrom<String> for Sid {
    type Error = InvalidSid;

    fn try_from(s: String) -> Result<Self, InvalidSid> {
        s.parse()
    }
}

impl From<Sid> for String {
    fn from(sid: Sid) -> String {
        sid.0
    }
}

impl fmt::Display for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S-1-5-21-1004336348-1177238915-682003330-512 (a Domain Admins SID).
    const DOMAIN_ADMINS: [u8; 28] = [
        1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 0xdc, 0xf4, 0xdc, 0x3b, 0x83, 0x3d, 0x2b, 0x46, 0x82,
        0x8b, 0xa6, 0x28, 0x00, 0x02, 0x00, 0x00,
    ];

    #[test]
    fn parses_the_binary_form() {
        let sid = Sid::from_bytes(&DOMAIN_ADMINS).unwrap();
        assert_eq!(sid.as_str(), "S-1-5-21-1004336348-1177238915-682003330-512");
        assert_eq!(sid.rid(), Some(512));
        // Well-known SID without sub-authorities beyond one: S-1-5-11 (Authenticated Users).
        assert_eq!(
            Sid::from_bytes(&[1, 1, 0, 0, 0, 0, 0, 5, 11, 0, 0, 0])
                .unwrap()
                .as_str(),
            "S-1-5-11"
        );
    }

    #[test]
    fn writes_the_binary_form_back() {
        let sid = Sid::from_bytes(&DOMAIN_ADMINS).unwrap();
        assert_eq!(sid.to_bytes(), DOMAIN_ADMINS);
        let short: Sid = "S-1-5-11".parse().unwrap();
        assert_eq!(short.to_bytes(), [1, 1, 0, 0, 0, 0, 0, 5, 11, 0, 0, 0]);
    }

    #[test]
    fn rejects_malformed_binary() {
        assert_eq!(Sid::from_bytes(&[]), Err(InvalidSid));
        assert_eq!(Sid::from_bytes(&DOMAIN_ADMINS[..27]), Err(InvalidSid));
        let mut wrong_revision = DOMAIN_ADMINS;
        wrong_revision[0] = 2;
        assert_eq!(Sid::from_bytes(&wrong_revision), Err(InvalidSid));
    }

    #[test]
    fn parses_and_validates_the_string_form() {
        let sid: Sid = "S-1-5-21-1-2-3-1105".parse().unwrap();
        assert_eq!(sid.to_string(), "S-1-5-21-1-2-3-1105");
        for bad in [
            "",
            "S-1",
            "S-2-5-21",
            "X-1-5",
            "S-1-5-abc",
            "S-1-5-99999999999",
        ] {
            assert!(bad.parse::<Sid>().is_err(), "{bad}");
        }
    }

    #[test]
    fn serialises_as_a_string_and_validates_on_deserialise() {
        let sid: Sid = "S-1-5-32-544".parse().unwrap();
        assert_eq!(serde_json::to_string(&sid).unwrap(), r#""S-1-5-32-544""#);
        assert!(serde_json::from_str::<Sid>(r#""not a sid""#).is_err());
    }
}
