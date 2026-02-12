use rusqlite::{Connection, params};

use crate::errors::AppError;

const SCHEMA_VERSION: i64 = 1;
const SCHEMA_V1_SQL: &str = include_str!("sql/schema_v1.sql");

pub fn apply(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "create table if not exists schema_migrations (
            version integer primary key,
            applied_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
    )
    .map_err(AppError::db_write)?;

    let already_applied: i64 = conn
        .query_row(
            "select count(*) from schema_migrations where version = ?1",
            params![SCHEMA_VERSION],
            |row| row.get(0),
        )
        .map_err(AppError::db_query)?;

    if already_applied == 0 {
        conn.execute_batch(SCHEMA_V1_SQL)
            .map_err(AppError::db_write)?;
        conn.execute(
            "insert into schema_migrations(version) values(?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(AppError::db_write)?;
    }

    Ok(())
}
