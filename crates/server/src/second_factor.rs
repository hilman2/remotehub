//! A second factor for directory sign-ins (#107): an authenticator app whose
//! secret remotehub keeps sealed in the vault (owner = user, field `totp`),
//! or security keys and passkeys (#129, `crate::webauthn`). Local accounts
//! get theirs from Kratos, break-glass accounts have one of their own.
//!
//! A user sets it up on their account page, or during sign-in when a rule in
//! `second_factor_principals` names them or one of their groups. Once set
//! up, every sign-in asks for a code or a key; each code works once, and
//! each key's challenge too.

use remotehub_vault::DynVault;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::problem::{ErrorCode, Problem};
use crate::secrets;
use crate::totp;
use crate::webauthn::{self, Assertion, RelyingParty};

const FIELD: &str = "totp";

/// What a sign-in's second step came to.
pub enum Outcome {
    /// No factor needed, or the code was right; `enrolled` if the app was
    /// set up just now.
    Passed { used: bool, enrolled: bool },
    /// The answer for the client; `failed` if it counts as a wrong attempt.
    Refused { problem: Problem, failed: bool },
}

/// Whether the user has an authenticator app or a security key (#129).
pub async fn enrolled<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM second_factors WHERE user_id = $1)
             OR EXISTS (SELECT 1 FROM security_keys WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// Whether the user has an authenticator app.
pub async fn has_app<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM second_factors WHERE user_id = $1)")
        .bind(user_id)
        .fetch_one(db)
        .await
}

/// Whether a rule names one of `sids` (principal and directory groups) or a
/// group of remotehub's own that one of them is in.
pub async fn required<'e>(db: impl PgExecutor<'e>, sids: &[String]) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM second_factor_principals p
             WHERE p.principal_sid = ANY ($1)
                OR p.principal_sid IN (SELECT 'group:' || m.group_id FROM group_members m
                                       WHERE m.principal_sid = ANY ($1)))",
    )
    .bind(sids)
    .fetch_one(db)
    .await
}

/// Checks a code of the user's app; a right one is used up.
pub async fn verify(
    tx: &mut PgConnection,
    vault: &DynVault,
    user_id: Uuid,
    code: &str,
    unix_seconds: u64,
) -> Result<bool, Problem> {
    let version: Option<i32> =
        sqlx::query_scalar("SELECT totp_version FROM second_factors WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(version) = version else {
        return Ok(false);
    };
    let secret = secrets::load(&mut *tx, vault, user_id, version, FIELD)
        .await
        .map_err(sealed)?
        .ok_or(Problem::new(ErrorCode::Internal))?;
    let Some(step) = totp::step(&secret, code.trim(), unix_seconds) else {
        return Ok(false);
    };
    let used = sqlx::query(
        "UPDATE second_factors SET last_totp_step = $2 WHERE user_id = $1 AND last_totp_step < $2",
    )
    .bind(user_id)
    .bind(i64::try_from(step).unwrap_or(i64::MAX))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(used == 1)
}

/// Sets up the app whose base32 `secret` the user was shown, if `code` is
/// one of its codes; a factor set up before is replaced.
pub async fn enroll(
    tx: &mut PgConnection,
    vault: &DynVault,
    user_id: Uuid,
    secret: &str,
    code: &str,
    unix_seconds: u64,
) -> Result<bool, Problem> {
    let secret = totp::decode(secret)
        .ok_or_else(|| Problem::new(ErrorCode::InvalidRequest).param("field", "totp_secret"))?;
    let Some(step) = totp::step(&secret, code.trim(), unix_seconds) else {
        return Ok(false);
    };
    let version: i32 = sqlx::query_scalar(
        "SELECT coalesce(max(version), 0) + 1 FROM secret_fields WHERE owner_id = $1 AND field = $2",
    )
    .bind(user_id)
    .bind(FIELD)
    .fetch_one(&mut *tx)
    .await?;
    secrets::store(&mut *tx, vault, user_id, version, FIELD, &secret)
        .await
        .map_err(sealed)?;
    sqlx::query(
        "INSERT INTO second_factors (user_id, totp_version, last_totp_step) VALUES ($1, $2, $3)
         ON CONFLICT (user_id) DO UPDATE
         SET totp_version = EXCLUDED.totp_version, last_totp_step = EXCLUDED.last_totp_step,
             created_at = now()",
    )
    .bind(user_id)
    .bind(version)
    .bind(i64::try_from(step).unwrap_or(i64::MAX))
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// Removes the user's authenticator app; whether they had one.
pub async fn remove<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM second_factors WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await?
        .rows_affected()
        == 1)
}

/// Removes every factor of the user, app and security keys, as an
/// administrator's reset does; whether there was one.
pub async fn remove_all(tx: &mut PgConnection, user_id: Uuid) -> sqlx::Result<bool> {
    let app = remove(&mut *tx, user_id).await?;
    let keys = sqlx::query("DELETE FROM security_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    Ok(app || keys > 0)
}

/// How long a challenge for a security key holds.
const CHALLENGE_SECONDS: f64 = 300.0;

/// A new challenge for `purpose` (`register` or `sign_in`); its ID and bytes.
pub async fn challenge<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    purpose: &str,
) -> sqlx::Result<(Uuid, [u8; 32])> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    // One statement, so a sign-in's challenge needs no transaction; the
    // expired ones go with it.
    let id = sqlx::query_scalar(
        "WITH gone AS (DELETE FROM webauthn_challenges WHERE expires_at < now())
         INSERT INTO webauthn_challenges (user_id, purpose, challenge, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4)) RETURNING id",
    )
    .bind(user_id)
    .bind(purpose)
    .bind(bytes.as_slice())
    .bind(CHALLENGE_SECONDS)
    .fetch_one(db)
    .await?;
    Ok((id, bytes))
}

/// The bytes of a challenge, used up by this call; none if it is not the
/// user's, for another purpose, used or expired.
pub async fn take_challenge<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    purpose: &str,
    id: Uuid,
) -> sqlx::Result<Option<Vec<u8>>> {
    sqlx::query_scalar(
        "DELETE FROM webauthn_challenges
         WHERE id = $1 AND user_id = $2 AND purpose = $3 AND expires_at > now()
         RETURNING challenge",
    )
    .bind(id)
    .bind(user_id)
    .bind(purpose)
    .fetch_optional(db)
    .await
}

/// The credential IDs of the user's security keys, base64url, for the
/// browser's `allowCredentials` or `excludeCredentials`.
pub async fn key_ids<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> sqlx::Result<Vec<String>> {
    use base64::Engine;
    let ids: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT credential_id FROM security_keys WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(ids
        .iter()
        .map(|id| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id))
        .collect())
}

/// What the browser needs to ask a security key for a signature (#129).
pub fn assertion_options(rp: &RelyingParty, challenge: &[u8], keys: &[String]) -> Value {
    use base64::Engine;
    json!({ "publicKey": {
        "challenge": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge),
        "rpId": rp.id,
        "timeout": 120_000,
        "userVerification": "discouraged",
        "allowCredentials": keys.iter()
            .map(|id| json!({ "type": "public-key", "id": id }))
            .collect::<Vec<_>>(),
    }})
}

/// Checks a security key's answer to a sign-in challenge; a right one moves
/// the key's counter on.
pub async fn verify_key(
    tx: &mut PgConnection,
    rp: &RelyingParty,
    user_id: Uuid,
    answer: &KeyAnswer,
) -> Result<bool, Problem> {
    let Some(challenge) = take_challenge(&mut *tx, user_id, "sign_in", answer.challenge_id).await?
    else {
        return Ok(false);
    };
    let Ok(credential_id) = answer.credential.credential_id() else {
        return Ok(false);
    };
    let key: Option<(Uuid, Vec<u8>, i64)> = sqlx::query_as(
        "SELECT id, public_key, sign_count FROM security_keys
         WHERE user_id = $1 AND credential_id = $2 FOR UPDATE",
    )
    .bind(user_id)
    .bind(&credential_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((key_id, public_key, count)) = key else {
        return Ok(false);
    };
    let stored = u32::try_from(count).unwrap_or(u32::MAX);
    match webauthn::verify(rp, &challenge, &public_key, stored, &answer.credential) {
        Ok(counter) => {
            sqlx::query(
                "UPDATE security_keys SET sign_count = $2, last_used_at = now() WHERE id = $1",
            )
            .bind(key_id)
            .bind(i64::from(counter))
            .execute(&mut *tx)
            .await?;
            Ok(true)
        }
        Err(error) => {
            tracing::warn!(%error, user = %user_id, "a security key's answer was refused");
            Ok(false)
        }
    }
}

/// A security key's answer at sign-in: the challenge it signed, and the
/// assertion.
#[derive(Debug, Deserialize)]
pub struct KeyAnswer {
    pub challenge_id: Uuid,
    pub credential: Assertion,
}

/// A new secret to show, and the URI an authenticator app reads.
pub fn offer(account: &str) -> (Zeroizing<String>, Zeroizing<String>) {
    let secret = totp::encode(totp::random_secret().as_slice());
    let uri = totp::uri(account, &secret);
    (secret, uri)
}

/// What a directory sign-in brings for its second step.
pub struct SecondStep<'a> {
    /// A code of the authenticator app.
    pub code: Option<&'a str>,
    /// The secret the server offered, when the app is set up now.
    pub new_secret: Option<&'a str>,
    /// A security key's answer (#129).
    pub key: Option<&'a KeyAnswer>,
}

/// Who signs in: the user, the name an authenticator app shows, and what
/// the rules may name them by.
pub struct Signer<'a> {
    pub user_id: Uuid,
    pub account: &'a str,
    pub sids: &'a [String],
}

/// The second step of a directory sign-in, after the password: a code of
/// the user's app or a security key's answer, or setting up the app when a
/// rule asks for it.
pub async fn at_sign_in(
    pool: &sqlx::PgPool,
    tx: &mut PgConnection,
    vault: &DynVault,
    rp: &RelyingParty,
    signer: Signer<'_>,
    step: SecondStep<'_>,
) -> Result<Outcome, Problem> {
    let Signer {
        user_id,
        account,
        sids,
    } = signer;
    let now = totp::unix_now();
    let wrong = || Outcome::Refused {
        problem: Problem::new(ErrorCode::SecondFactorInvalid),
        failed: true,
    };
    let passed = Outcome::Passed {
        used: true,
        enrolled: false,
    };
    if enrolled(&mut *tx, user_id).await? {
        if let Some(code) = step.code {
            return Ok(if verify(tx, vault, user_id, code, now).await? {
                passed
            } else {
                wrong()
            });
        }
        if let Some(key) = step.key {
            return Ok(if verify_key(tx, rp, user_id, key).await? {
                passed
            } else {
                wrong()
            });
        }
        // Asked for: the app's code, a key's answer, or either.
        let mut problem = Problem::new(ErrorCode::SecondFactorRequired)
            .param("app", has_app(&mut *tx, user_id).await?);
        let keys = key_ids(&mut *tx, user_id).await?;
        if !keys.is_empty() {
            // Outside the sign-in's transaction, which ends here unused.
            let (id, bytes) = challenge(pool, user_id, "sign_in").await?;
            problem = problem
                .param("key_challenge_id", id.to_string())
                .param("key_options", assertion_options(rp, &bytes, &keys));
        }
        return Ok(Outcome::Refused {
            problem,
            failed: false,
        });
    }
    let (code, new_secret) = (step.code, step.new_secret);
    if let (Some(secret), Some(code)) = (new_secret, code) {
        return Ok(if enroll(tx, vault, user_id, secret, code, now).await? {
            Outcome::Passed {
                used: true,
                enrolled: true,
            }
        } else {
            wrong()
        });
    }
    if required(&mut *tx, sids).await? {
        let (secret, uri) = offer(account);
        return Ok(Outcome::Refused {
            problem: Problem::new(ErrorCode::SecondFactorSetupRequired)
                .param("secret", secret.as_str())
                .param("uri", uri.as_str()),
            failed: false,
        });
    }
    Ok(Outcome::Passed {
        used: false,
        enrolled: false,
    })
}

fn sealed(error: secrets::SecretError) -> Problem {
    tracing::error!(%error, "a second factor's secret cannot be sealed or opened");
    Problem::new(ErrorCode::Internal)
}
