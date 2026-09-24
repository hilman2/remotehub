//! PostgreSQL pool and migrations.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

/// Forward-only migrations from `migrations/`, embedded in the binary and
/// applied at startup. sqlx takes an advisory lock, so several instances
/// starting at once do not collide.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
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
