//! PostgreSQL pool and migrations.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Forward-only migrations from `migrations/`, embedded in the binary and
/// applied at startup. sqlx takes an advisory lock, so several instances
/// starting at once do not collide.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Connects to `database_url`; `password`, if given, replaces the URL's.
pub async fn connect(
    database_url: &str,
    password: Option<&SecretString>,
) -> Result<PgPool, sqlx::Error> {
    let mut options: PgConnectOptions = database_url.parse()?;
    if let Some(password) = password {
        options = options.password(password.expose_secret());
    }
    PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
}

/// Whether the database answers within a short time; used by the health check.
pub async fn is_reachable(pool: &PgPool) -> bool {
    let probe = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool);
    matches!(
        tokio::time::timeout(Duration::from_secs(2), probe).await,
        Ok(Ok(1))
    )
}
