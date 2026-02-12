use rusqlite::{Connection, params};
use serde::Serialize;

use crate::errors::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryState {
    All,
    Pending,
    Enriched,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddedItem {
    pub item_id: i64,
    pub created_at: String,
    pub source: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListItem {
    pub item_id: i64,
    pub created_at: String,
    pub state: String,
    pub text_preview: String,
    pub content_type: Option<String>,
    pub validation_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchItem {
    pub item_id: i64,
    pub created_at: String,
    pub source: String,
    pub text: String,
    pub state: String,
    pub content_type: Option<String>,
    pub validation_status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FetchCursor {
    pub item_id: i64,
    pub created_at: String,
}

pub fn add_item(
    conn: &Connection,
    text: &str,
    source: &str,
    created_at: Option<&str>,
) -> Result<AddedItem, AppError> {
    if let Some(created_at) = created_at {
        conn.execute(
            "insert into inbox_items(source, raw_text, created_at) values(?1, ?2, ?3)",
            params![source, text, created_at],
        )
        .map_err(AppError::db_write)?;
    } else {
        conn.execute(
            "insert into inbox_items(source, raw_text) values(?1, ?2)",
            params![source, text],
        )
        .map_err(AppError::db_write)?;
    }

    let item_id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row(
            "select created_at from inbox_items where item_id = ?1",
            params![item_id],
            |row| row.get(0),
        )
        .map_err(AppError::db_query)?;

    Ok(AddedItem {
        item_id,
        created_at,
        source: source.to_string(),
        text: text.to_string(),
    })
}

pub fn list_items(
    conn: &Connection,
    state: QueryState,
    limit: usize,
    offset: usize,
) -> Result<Vec<ListItem>, AppError> {
    let state_filter = state_sql(state);
    let sql = format!(
        "select
            i.item_id,
            i.created_at,
            case
                when ad.derivation_id is not null then 'enriched'
                else 'pending'
            end as state,
            substr(i.raw_text, 1, 80) as text_preview,
            json_extract(ad.payload_json, '$.content_type') as content_type,
            json_extract(ad.payload_json, '$.validation_status') as validation_status
        from inbox_items i
        left join item_derivations ad
          on ad.derivation_id = (
            select d.derivation_id
            from item_derivations d
            where d.item_id = i.item_id
              and d.is_active = 1
              and d.status = 'accepted'
            order by d.derivation_version desc, d.derivation_id desc
            limit 1
          )
        where {state_filter}
        order by i.created_at desc, i.item_id desc
        limit ?1 offset ?2"
    );

    let mut stmt = conn.prepare(&sql).map_err(AppError::db_query)?;
    let rows = stmt
        .query_map(params![limit as i64, offset as i64], |row| {
            Ok(ListItem {
                item_id: row.get(0)?,
                created_at: row.get(1)?,
                state: row.get(2)?,
                text_preview: row.get(3)?,
                content_type: row.get(4)?,
                validation_status: row.get(5)?,
            })
        })
        .map_err(AppError::db_query)?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::db_query)
}

pub fn lookup_fetch_cursor(
    conn: &Connection,
    item_id: i64,
) -> Result<Option<FetchCursor>, AppError> {
    conn.query_row(
        "select item_id, created_at from inbox_items where item_id = ?1",
        params![item_id],
        |row| {
            Ok(FetchCursor {
                item_id: row.get(0)?,
                created_at: row.get(1)?,
            })
        },
    )
    .map(Some)
    .or_else(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(AppError::db_query(other)),
    })
}

pub fn fetch_pending_page(
    conn: &Connection,
    limit: usize,
    cursor: Option<&FetchCursor>,
) -> Result<Vec<FetchItem>, AppError> {
    let mut stmt = conn
        .prepare(
            "select i.item_id, i.created_at, i.source, i.raw_text
                    , null as content_type
                    , null as validation_status
            from inbox_items i
            where not exists (
                select 1 from item_derivations d
                where d.item_id = i.item_id and d.is_active = 1 and d.status = 'accepted'
            )
              and (
                ?1 is null
                or i.created_at < ?2
                or (i.created_at = ?2 and i.item_id < ?1)
              )
            order by i.created_at desc, i.item_id desc
            limit ?3",
        )
        .map_err(AppError::db_query)?;

    let cursor_item_id = cursor.map(|value| value.item_id);
    let cursor_created_at = cursor.map(|value| value.created_at.as_str());

    let rows = stmt
        .query_map(
            params![cursor_item_id, cursor_created_at, limit as i64],
            |row| {
                Ok(FetchItem {
                    item_id: row.get(0)?,
                    created_at: row.get(1)?,
                    source: row.get(2)?,
                    text: row.get(3)?,
                    state: "pending".to_string(),
                    content_type: row.get(4)?,
                    validation_status: row.get(5)?,
                })
            },
        )
        .map_err(AppError::db_query)?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::db_query)
}

fn state_sql(state: QueryState) -> &'static str {
    match state {
        QueryState::All => "1 = 1",
        QueryState::Pending => {
            "not exists (
                select 1 from item_derivations d
                where d.item_id = i.item_id and d.is_active = 1 and d.status = 'accepted'
            )"
        }
        QueryState::Enriched => {
            "exists (
                select 1 from item_derivations d
                where d.item_id = i.item_id and d.is_active = 1 and d.status = 'accepted'
            )"
        }
    }
}
