use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[sqlx(type_name = "TEXT", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AiRoomStorageMode {
    LocalOnly,
    TaskAiCloud,
    PersonalHub,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct AiRoomLocalIdentity {
    pub owner_id: Uuid,
    pub device_id: Uuid,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct AiRoomStorageProfile {
    pub room_id: Uuid,
    pub owner_id: Uuid,
    pub mode: AiRoomStorageMode,
    pub endpoint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

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
    #[serde(default)]
    pub allow_existing_local_root: bool,
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

    pub async fn set_instruction_version(
        pool: &SqlitePool,
        id: Uuid,
        instruction_version: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE ai_rooms SET instruction_version = ?, updated_at = datetime('now', 'subsec') WHERE id = ? AND instruction_version < ?",
        )
        .bind(instruction_version)
        .bind(id)
        .bind(instruction_version)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn update_profile(
        pool: &SqlitePool,
        id: Uuid,
        name: String,
        description: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query(
            "UPDATE ai_rooms SET name = ?, description = ?, updated_at = datetime('now', 'subsec') WHERE id = ?",
        )
        .bind(name)
        .bind(description)
        .bind(id)
        .execute(pool)
        .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn update_connection(
        pool: &SqlitePool,
        id: Uuid,
        ssh_alias: Option<String>,
        remote_root: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query(
            "UPDATE ai_rooms SET ssh_alias = ?, remote_root = ?, updated_at = datetime('now', 'subsec') WHERE id = ?",
        )
        .bind(ssh_alias)
        .bind(remote_root)
        .bind(id)
        .execute(pool)
        .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
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

impl AiRoomLocalIdentity {
    pub async fn ensure(pool: &SqlitePool) -> Result<Self, sqlx::Error> {
        if let Some(identity) = sqlx::query_as::<_, Self>(
            "SELECT owner_id, device_id, device_name, created_at, updated_at
             FROM ai_room_local_identity
             WHERE singleton = 1",
        )
        .fetch_optional(pool)
        .await?
        {
            return Ok(identity);
        }

        let owner_id = Uuid::new_v4();
        let device_id = Uuid::new_v4();
        sqlx::query(
            "INSERT OR IGNORE INTO ai_room_local_identity
             (singleton, owner_id, device_id, device_name)
             VALUES (1, ?, ?, '이 PC')",
        )
        .bind(owner_id)
        .bind(device_id)
        .execute(pool)
        .await?;

        sqlx::query_as::<_, Self>(
            "SELECT owner_id, device_id, device_name, created_at, updated_at
             FROM ai_room_local_identity
             WHERE singleton = 1",
        )
        .fetch_one(pool)
        .await
    }
}

impl AiRoomStorageProfile {
    pub async fn ensure(
        pool: &SqlitePool,
        room_id: Uuid,
    ) -> Result<(AiRoomLocalIdentity, Self), sqlx::Error> {
        let identity = AiRoomLocalIdentity::ensure(pool).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO ai_room_storage_profiles
             (room_id, owner_id, mode)
             VALUES (?, ?, 'LOCAL_ONLY')",
        )
        .bind(room_id)
        .bind(identity.owner_id)
        .execute(pool)
        .await?;

        let profile = sqlx::query_as::<_, Self>(
            "SELECT room_id, owner_id, mode, endpoint, created_at, updated_at
             FROM ai_room_storage_profiles
             WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_one(pool)
        .await?;

        Ok((identity, profile))
    }
}
