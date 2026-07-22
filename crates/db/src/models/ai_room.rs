use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct AiRoom {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub local_root: String,
    pub ssh_alias: Option<String>,
    pub remote_root: Option<String>,
    pub instruction_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateAiRoom {
    pub name: String,
    pub description: Option<String>,
    pub local_root: String,
    pub ssh_alias: Option<String>,
    pub remote_root: Option<String>,
}

impl AiRoom {
    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM ai_rooms ORDER BY updated_at DESC")
            .fetch_all(pool)
            .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM ai_rooms WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(pool: &SqlitePool, data: CreateAiRoom) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ai_rooms (id, name, description, local_root, ssh_alias, remote_root) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(data.name)
        .bind(data.description)
        .bind(data.local_root)
        .bind(data.ssh_alias)
        .bind(data.remote_root)
        .execute(pool)
        .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn touch(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE ai_rooms SET updated_at = datetime('now', 'subsec') WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM ai_rooms WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0)
    }
}
