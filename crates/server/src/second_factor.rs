//! A second factor for directory sign-ins (#107): an authenticator app whose
//! secret remotehub keeps sealed in the vault (owner = user, field `totp`).
//! Local accounts get theirs from Kratos, break-glass accounts have one of
//! their own.
//!
//! A user sets it up on their account page, or during sign-in when a rule in
//! `second_factor_principals` names them or one of their groups. Once set
//! up, every sign-in asks for a code, and each code works once.

use remotehub_vault::DynVault;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::problem::{ErrorCode, Problem};
use crate::secrets;
use crate::totp;

const FIELD: &str = "totp";

/// What a sign-in's second step came to.
pub enum Outcome {
    /// No factor needed, or the code was right; `enrolled` if the app was
    /// set up just now.
    Passed { used: bool, enrolled: bool },
    /// The answer for the client; `failed` if it counts as a wrong attempt.
    Refused { problem: Problem, failed: bool },
}

pub async fn enrolled<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> sqlx::Result<bool> {
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

/// Removes the user's factor; whether they had one.
pub async fn remove<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM second_factors WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
        .await?
        .rows_affected()
        == 1)
}

/// A new secret to show, and the URI an authenticator app reads.
pub fn offer(account: &str) -> (Zeroizing<String>, Zeroizing<String>) {
    let secret = totp::encode(totp::random_secret().as_slice());
    let uri = totp::uri(account, &secret);
    (secret, uri)
}

/// The second step of a directory sign-in, after the password: a code of
/// the user's app, or setting one up when a rule asks for it.
pub async fn at_sign_in(
    tx: &mut PgConnection,
    vault: &DynVault,
    user_id: Uuid,
    account: &str,
    sids: &[String],
    code: Option<&str>,
    new_secret: Option<&str>,
) -> Result<Outcome, Problem> {
    let now = totp::unix_now();
    let wrong = || Outcome::Refused {
        problem: Problem::new(ErrorCode::SecondFactorInvalid),
        failed: true,
    };
    if enrolled(&mut *tx, user_id).await? {
        return Ok(match code {
            None => Outcome::Refused {
                problem: Problem::new(ErrorCode::SecondFactorRequired),
                failed: false,
            },
            Some(code) if verify(tx, vault, user_id, code, now).await? => Outcome::Passed {
                used: true,
                enrolled: false,
            },
            Some(_) => wrong(),
        });
    }
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
