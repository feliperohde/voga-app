//! The local SQLite database (voga.sqlite in the app data folder), migrated on every start.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

const FILE: &str = "voga.sqlite";

#[derive(Clone)]
pub struct DatabaseManager {
    pool: SqlitePool,
}

impl DatabaseManager {
    pub async fn open(app_data_dir: &Path) -> Result<Self, sqlx::Error> {
        std::fs::create_dir_all(app_data_dir).map_err(sqlx::Error::Io)?;
        let path = app_data_dir.join(FILE);
        log::info!("Database: {}", path.display());
        let pool = match Self::connect(&path).await {
            Ok(pool) => pool,
            // A WAL left by a crash can make the file look corrupt: drop it and retry once
            Err(e) if e.to_string().contains("malformed") || e.to_string().contains("corrupt") => {
                log::warn!("Database looks corrupt ({e}); removing its WAL files and retrying");
                for suffix in ["-wal", "-shm"] {
                    let _ = std::fs::remove_file(app_data_dir.join(format!("{FILE}{suffix}")));
                }
                Self::connect(&path).await?
            }
            Err(e) => return Err(e),
        };
        Ok(Self { pool })
    }

    async fn connect(path: &Path) -> Result<SqlitePool, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(&path.to_string_lossy())?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(pool)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Writes the WAL into the main file and closes the pool (on exit).
    pub async fn cleanup(&self) {
        if let Err(e) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&self.pool).await {
            log::warn!("WAL checkpoint failed: {e}");
        }
        self.pool.close().await;
    }
}
