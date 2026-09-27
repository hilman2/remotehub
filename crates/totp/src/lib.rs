//! TOTP (RFC 6238, HMAC-SHA1, six digits, 30 seconds) for break-glass
//! accounts, the second factor of directory users (#107) and the users of a
//! site connector's web interface (#165).
//!
//! [`Params`] computes the codes of any service's one-time password as its
//! `otpauth://` link describes it, for entries of the vault (#193).

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

pub const PERIOD: u64 = 30;
pub const DIGITS: u32 = 6;
/// 160 bits, as RFC 4226 recommends.
pub const SECRET_BYTES: usize = 20;

/// The code for a time step.
pub fn code(secret: &[u8], step: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let value = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]) % 10u32.pow(DIGITS);
    format!("{value:0width$}", width = DIGITS as usize)
}

/// The time step a code belongs to, allowing one step of clock drift each way.
pub fn step(secret: &[u8], code: &str, unix_seconds: u64) -> Option<u64> {
    let now = unix_seconds / PERIOD;
    [now, now.saturating_sub(1), now + 1]
        .into_iter()
        .find(|step| {
            let expected = self::code(secret, *step);
            // Constant-time comparison of equal-length ASCII codes.
            expected.len() == code.len()
                && expected
                    .bytes()
                    .zip(code.bytes())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0
        })
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs()
}

pub fn random_secret() -> Zeroizing<[u8; SECRET_BYTES]> {
    let mut secret = Zeroizing::new([0u8; SECRET_BYTES]);
    getrandom::fill(secret.as_mut_slice()).expect("the operating system provides randomness");
    secret
}

/// Base32 without padding, as authenticator apps take it.
pub fn encode(secret: &[u8]) -> Zeroizing<String> {
    Zeroizing::new(BASE32_NOPAD.encode(secret))
}

/// A base32 secret of the right length, as [`encode`] writes it.
pub fn decode(secret: &str) -> Option<Zeroizing<Vec<u8>>> {
    let bytes = Zeroizing::new(BASE32_NOPAD.decode(secret.trim().as_bytes()).ok()?);
    (bytes.len() == SECRET_BYTES).then_some(bytes)
}

/// The `otpauth://` URI an authenticator app reads from a QR code; the app
/// lists the account under `issuer`.
pub fn uri(issuer: &str, account: &str, secret_base32: &str) -> Zeroizing<String> {
    Zeroizing::new(format!(
        "otpauth://totp/{issuer}:{account}?secret={secret_base32}&issuer={issuer}&algorithm=SHA1&digits={DIGITS}&period={PERIOD}"
    ))
}

/// The hash function of a service's codes, as its `otpauth://` link names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

/// A service's one-time password: its secret and how codes are made from it.
#[derive(Clone, PartialEq, Eq)]
pub struct Params {
    pub secret: Zeroizing<Vec<u8>>,
    pub algorithm: Algorithm,
    /// 6 to 8.
    pub digits: u32,
    /// Seconds a code holds, 1 to 300.
    pub period: u64,
}

/// Everything but the secret, so a log never shows it.
impl std::fmt::Debug for Params {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Params")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .finish_non_exhaustive()
    }
}

/// Shorter secrets are typing errors, not secrets: 80 bits is the least that
/// services hand out.
const MIN_SECRET_BYTES: usize = 10;

impl Params {
    /// Reads an `otpauth://totp/…` link, or a bare base32 secret with the
    /// usual SHA-1, six digits and 30 seconds. None for anything else,
    /// HOTP links included.
    pub fn parse(text: &str) -> Option<Params> {
        let text = text.trim();
        let Some(rest) = strip_prefix_ignore_case(text, "otpauth://totp/") else {
            return Some(Params {
                secret: lenient_base32(text)?,
                algorithm: Algorithm::Sha1,
                digits: DIGITS,
                period: PERIOD,
            });
        };
        let query = rest.split_once('?')?.1;
        let mut params = Params {
            secret: Zeroizing::new(Vec::new()),
            algorithm: Algorithm::Sha1,
            digits: DIGITS,
            period: PERIOD,
        };
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key.to_ascii_lowercase().as_str() {
                "secret" => params.secret = lenient_base32(value)?,
                "algorithm" => {
                    params.algorithm = match value.to_ascii_uppercase().as_str() {
                        "SHA1" => Algorithm::Sha1,
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        _ => return None,
                    }
                }
                "digits" => params.digits = value.parse().ok().filter(|d| (6..=8).contains(d))?,
                "period" => {
                    params.period = value.parse().ok().filter(|p| (1..=300).contains(p))?;
                }
                // Issuer, image and the like name the account, not the code.
                _ => {}
            }
        }
        (!params.secret.is_empty()).then_some(params)
    }

    /// The code at `unix_seconds`, and how many seconds it still holds.
    pub fn code_at(&self, unix_seconds: u64) -> (String, u64) {
        let step = unix_seconds / self.period;
        let digest = match self.algorithm {
            Algorithm::Sha1 => hmac::<Hmac<Sha1>>(&self.secret, step),
            Algorithm::Sha256 => hmac::<Hmac<Sha256>>(&self.secret, step),
            Algorithm::Sha512 => hmac::<Hmac<Sha512>>(&self.secret, step),
        };
        let offset = usize::from(digest[digest.len() - 1] & 0x0f);
        let value = u32::from_be_bytes([
            digest[offset] & 0x7f,
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
        ]) % 10u32.pow(self.digits);
        let code = format!("{value:0width$}", width = self.digits as usize);
        (code, self.period - unix_seconds % self.period)
    }
}

fn hmac<M: Mac + KeyInit>(key: &[u8], step: u64) -> Vec<u8> {
    let mut mac = <M as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Base32 as people copy it from a service: any case, with spaces, dashes or
/// padding; at least 80 bits.
fn lenient_base32(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let bytes = Zeroizing::new(BASE32_NOPAD.decode(clean.as_bytes()).ok()?);
    (bytes.len() >= MIN_SECRET_BYTES).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, appendix B, all three hash functions with eight digits.
    #[test]
    fn params_match_the_rfc_test_vectors() {
        let seeds: [(Algorithm, &[u8]); 3] = [
            (Algorithm::Sha1, b"12345678901234567890"),
            (Algorithm::Sha256, b"12345678901234567890123456789012"),
            (
                Algorithm::Sha512,
                b"1234567890123456789012345678901234567890123456789012345678901234",
            ),
        ];
        let expected = [
            (59, ["94287082", "46119246", "90693936"]),
            (1_111_111_109, ["07081804", "68084774", "25091201"]),
            (2_000_000_000, ["69279037", "90698825", "38618901"]),
        ];
        for (time, codes) in expected {
            for ((algorithm, seed), code) in seeds.iter().zip(codes) {
                let params = Params {
                    secret: Zeroizing::new(seed.to_vec()),
                    algorithm: *algorithm,
                    digits: 8,
                    period: 30,
                };
                assert_eq!(params.code_at(time).0, code, "{algorithm:?} at {time}");
            }
        }
    }

    #[test]
    fn links_and_bare_secrets_are_read() {
        let secret = encode(b"12345678901234567890123456789012");
        let link = format!(
            "otpauth://totp/Shop:alice?secret={}&issuer=Shop&algorithm=SHA256&digits=8&period=60",
            secret.to_lowercase()
        );
        let params = Params::parse(&link).unwrap();
        assert_eq!(params.algorithm, Algorithm::Sha256);
        assert_eq!((params.digits, params.period), (8, 60));
        assert_eq!(
            params.secret.as_slice(),
            b"12345678901234567890123456789012"
        );
        // A code of 60 seconds holds until the next full minute.
        assert_eq!(params.code_at(59 * 60 + 15).1, 45);

        let bare = Params::parse(" jbsw y3dp ehpk 3pxp ").unwrap();
        assert_eq!(
            (bare.algorithm, bare.digits, bare.period),
            (Algorithm::Sha1, 6, 30)
        );
        assert_eq!(bare.secret.len(), 10);
    }

    #[test]
    fn what_is_no_totp_is_refused() {
        for text in [
            "",
            "not base32!",
            "JBSWY3DP",
            "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=9",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
            "otpauth://totp/x?issuer=nothing",
        ] {
            assert_eq!(Params::parse(text), None, "{text}");
        }
    }

    /// RFC 6238, appendix B (SHA-1, 8 digits there; the last 6 digits here).
    #[test]
    fn matches_the_rfc_test_vectors() {
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
        ] {
            assert_eq!(code(secret, time / PERIOD), expected, "{time}");
        }
    }

    #[test]
    fn codes_are_accepted_one_step_around_now() {
        let secret = b"12345678901234567890";
        let now = 1_234_567_890;
        let current = code(secret, now / 30);
        assert_eq!(step(secret, &current, now), Some(now / 30));
        let previous = code(secret, now / 30 - 1);
        assert_eq!(step(secret, &previous, now), Some(now / 30 - 1));
        let old = code(secret, now / 30 - 2);
        assert_eq!(step(secret, &old, now), None);
        assert_eq!(step(secret, "12345", now), None);
    }

    #[test]
    fn secrets_are_read_back_only_at_full_length() {
        let secret = random_secret();
        let encoded = encode(secret.as_slice());
        assert_eq!(decode(&encoded).unwrap().as_slice(), secret.as_slice());
        assert!(
            decode("JBSWY3DPEHPK3PXP").is_none(),
            "80 bits are too short"
        );
        assert!(decode("not base32!").is_none());
    }
}
