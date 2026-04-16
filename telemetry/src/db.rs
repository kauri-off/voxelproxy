use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;

pub async fn init_db() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite://telemetry.db")
        .unwrap()
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(opts).await.unwrap();

    sqlx::query("CREATE TABLE IF NOT EXISTS pings (id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, ip TEXT NOT NULL, version TEXT NOT NULL, os TEXT NOT NULL, username TEXT NOT NULL);")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("CREATE TABLE IF NOT EXISTS starts (id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, ip TEXT NOT NULL, type TEXT NOT NULL, username TEXT NOT NULL, server_addr TEXT, windivert INTEGER);")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("CREATE TABLE IF NOT EXISTS auto_joins (id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, ip TEXT NOT NULL, username TEXT NOT NULL, server_addr TEXT);").execute(&pool).await.unwrap();

    pool
}
