//! TOTP (RFC 6238, HMAC-SHA1, six digits, 30 seconds) for break-glass
//! accounts, the second factor of directory users (#107) and the users of a
//! site connector's web interface (#165).

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
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

#[cfg(test)]
mod tests {
    use super::*;

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
