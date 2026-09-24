//! Who may sign in: the configured directory behind a dyn-compatible trait,
//! plus the limit on failed attempts.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use remotehub_directory::{AuthError, Identity, IdentityProvider};
use secrecy::SecretString;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// [`IdentityProvider`] as a trait object, so the server can hold whichever
/// directory is configured (and tests a fake one).
pub trait Authenticator: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        username: &'a str,
        password: &'a SecretString,
    ) -> BoxFuture<'a, Result<Identity, AuthError>>;
}

impl<T: IdentityProvider> Authenticator for T {
    fn authenticate<'a>(
        &'a self,
        username: &'a str,
        password: &'a SecretString,
    ) -> BoxFuture<'a, Result<Identity, AuthError>> {
        Box::pin(IdentityProvider::authenticate(self, username, password))
    }
}

/// Counts failed sign-ins per key (user name, client address) in a sliding
/// window. Blocked keys wait until the window of their first failure ends.
pub struct SignInLimiter {
    windows: Mutex<HashMap<String, Window>>,
    window: Duration,
}

#[derive(Clone, Copy)]
struct Window {
    failures: u32,
    since: Instant,
}

/// How many failures a key may have per window.
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub key_kind: &'static str,
    pub max_failures: u32,
}

pub const PER_USER: Limit = Limit {
    key_kind: "user",
    max_failures: 5,
};
pub const PER_ADDRESS: Limit = Limit {
    key_kind: "address",
    max_failures: 30,
};

impl SignInLimiter {
    pub fn new(window: Duration) -> Self {
        SignInLimiter {
            windows: Mutex::new(HashMap::new()),
            window,
        }
    }

    fn key(limit: Limit, value: &str) -> String {
        format!("{}:{}", limit.key_kind, value.trim().to_lowercase())
    }

    /// `Err(wait)` if any key has used up its failures.
    pub fn check(&self, keys: &[(Limit, &str)]) -> Result<(), Duration> {
        let now = Instant::now();
        let windows = self.windows.lock().expect("limiter lock");
        let wait = keys
            .iter()
            .filter_map(|(limit, value)| {
                let window = windows.get(&Self::key(*limit, value))?;
                let elapsed = now.duration_since(window.since);
                (elapsed < self.window && window.failures >= limit.max_failures)
                    .then(|| self.window - elapsed)
            })
            .max();
        match wait {
            Some(wait) => Err(wait.max(Duration::from_secs(1))),
            None => Ok(()),
        }
    }

    pub fn failed(&self, keys: &[(Limit, &str)]) {
        let now = Instant::now();
        let mut windows = self.windows.lock().expect("limiter lock");
        if windows.len() > 10_000 {
            windows.retain(|_, w| now.duration_since(w.since) < self.window);
        }
        for (limit, value) in keys {
            let window = windows.entry(Self::key(*limit, value)).or_insert(Window {
                failures: 0,
                since: now,
            });
            if now.duration_since(window.since) >= self.window {
                *window = Window {
                    failures: 0,
                    since: now,
                };
            }
            window.failures += 1;
        }
    }

    /// A successful sign-in clears the user's failures (not the address's).
    pub fn succeeded(&self, user: &str) {
        self.windows
            .lock()
            .expect("limiter lock")
            .remove(&Self::key(PER_USER, user));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_the_limit_and_clears_on_success() {
        let limiter = SignInLimiter::new(Duration::from_secs(300));
        let keys = [(PER_USER, "Alice"), (PER_ADDRESS, "10.0.0.1")];
        for _ in 0..PER_USER.max_failures {
            assert!(limiter.check(&keys).is_ok());
            limiter.failed(&keys);
        }
        let wait = limiter.check(&keys).unwrap_err();
        assert!(wait > Duration::from_secs(290) && wait <= Duration::from_secs(300));
        // Names are compared case-insensitively.
        assert!(limiter.check(&[(PER_USER, " alice ")]).is_err());
        // Another user from the same address is still allowed.
        assert!(
            limiter
                .check(&[(PER_USER, "bob"), (PER_ADDRESS, "10.0.0.1")])
                .is_ok()
        );

        limiter.succeeded("alice");
        assert!(limiter.check(&keys).is_ok());
    }

    #[test]
    fn windows_expire() {
        let limiter = SignInLimiter::new(Duration::from_millis(20));
        let keys = [(PER_USER, "alice")];
        for _ in 0..PER_USER.max_failures {
            limiter.failed(&keys);
        }
        assert!(limiter.check(&keys).is_err());
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.check(&keys).is_ok());
    }
}
