use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool, types::Json};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ProviderPlugin {
    pub id: Uuid,
    pub plugin_key: String,
    pub display_name: String,
    pub version: String,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub manifest_path: String,
    pub adapter_entrypoint: String,
    #[ts(type = "unknown")]
    pub configuration_schema: Json<Value>,
    #[ts(type = "unknown")]
    pub credential_schema: Json<Value>,
    #[ts(type = "Array<string>")]
    pub capabilities: Json<Vec<String>>,
    #[ts(type = "Array<string>")]
    pub runtime_types: Json<Vec<String>>,
    #[ts(type = "Array<string>")]
    pub permissions: Json<Vec<String>>,
    pub status: String,
    pub enabled: bool,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateProviderPlugin {
    pub plugin_key: String,
    pub display_name: String,
    pub version: String,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub manifest_path: String,
    pub adapter_entrypoint: String,
    #[ts(type = "unknown")]
    pub configuration_schema: Value,
    #[ts(type = "unknown")]
    pub credential_schema: Value,
    pub capabilities: Vec<String>,
    pub runtime_types: Vec<String>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ProviderAccount {
    pub id: Uuid,
    pub provider_plugin_id: Uuid,
    pub display_name: String,
    pub account_type: String,
    pub credential_reference: Option<String>,
    #[ts(type = "unknown")]
    pub configuration_json: Json<Value>,
    pub status: String,
    pub enabled: bool,
    pub last_validated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateProviderAccount {
    pub provider_plugin_id: Uuid,
    pub display_name: String,
    pub account_type: String,
    pub credential_reference: Option<String>,
    #[ts(type = "unknown")]
    pub configuration_json: Value,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ModelDefinition {
    pub id: Uuid,
    pub provider_account_id: Uuid,
    pub provider_model_key: String,
    pub display_name: String,
    pub description: Option<String>,
    pub context_window: Option<i64>,
    #[ts(type = "Array<string>")]
    pub input_modalities: Json<Vec<String>>,
    #[ts(type = "Array<string>")]
    pub output_modalities: Json<Vec<String>>,
    #[ts(type = "Array<string>")]
    pub capability_json: Json<Vec<String>>,
    #[ts(type = "unknown")]
    pub pricing_json: Json<Value>,
    pub availability_status: String,
    pub discovered_automatically: bool,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateModelDefinition {
    pub provider_account_id: Uuid,
    pub provider_model_key: String,
    pub display_name: String,
    pub description: Option<String>,
    pub context_window: Option<i64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub capabilities: Vec<String>,
    #[ts(type = "unknown")]
    pub pricing_json: Value,
    #[serde(default)]
    pub discovered_automatically: bool,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct RuntimeProfile {
    pub id: Uuid,
    pub provider_account_id: Uuid,
    pub name: String,
    pub runtime_type: String,
    pub executable_path: Option<String>,
    pub endpoint: Option<String>,
    pub remote_connection_id: Option<String>,
    pub working_directory_policy: String,
    pub environment_reference: Option<String>,
    #[ts(type = "unknown")]
    pub configuration_json: Json<Value>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateRuntimeProfile {
    pub provider_account_id: Uuid,
    pub name: String,
    pub runtime_type: String,
    pub executable_path: Option<String>,
    pub endpoint: Option<String>,
    pub remote_connection_id: Option<String>,
    pub working_directory_policy: String,
    pub environment_reference: Option<String>,
    #[ts(type = "unknown")]
    pub configuration_json: Value,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ProviderPermissionPolicy {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    #[ts(type = "Array<string>")]
    pub approved_permissions: Json<Vec<String>>,
    #[ts(type = "unknown")]
    pub constraints_json: Json<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateProviderPermissionPolicy {
    pub name: String,
    pub description: Option<String>,
    pub approved_permissions: Vec<String>,
    #[ts(type = "unknown")]
    pub constraints_json: Value,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct AgentProfile {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub role_key: String,
    pub system_instructions: String,
    #[ts(type = "Array<string>")]
    pub compatible_capabilities: Json<Vec<String>>,
    pub preferred_provider_plugin_id: Option<Uuid>,
    pub preferred_model_id: Option<Uuid>,
    #[ts(type = "Array<string>")]
    pub allowed_tools: Json<Vec<String>>,
    #[ts(type = "Array<string>")]
    pub denied_tools: Json<Vec<String>>,
    pub permission_policy_id: Option<Uuid>,
    #[ts(type = "unknown")]
    pub context_policy_json: Json<Value>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateAgentProfile {
    pub name: String,
    pub description: Option<String>,
    pub role_key: String,
    pub system_instructions: String,
    pub compatible_capabilities: Vec<String>,
    pub preferred_provider_plugin_id: Option<Uuid>,
    pub preferred_model_id: Option<Uuid>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub permission_policy_id: Option<Uuid>,
    #[ts(type = "unknown")]
    pub context_policy_json: Value,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct Assignment {
    pub id: Uuid,
    pub task_id: String,
    pub provider_plugin_id: Uuid,
    pub provider_account_id: Uuid,
    pub model_definition_id: Option<Uuid>,
    pub runtime_profile_id: Uuid,
    pub agent_profile_id: Uuid,
    pub permission_policy_id: Option<Uuid>,
    #[ts(type = "Array<string>")]
    pub required_capabilities: Json<Vec<String>>,
    pub status: String,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub handoff_from_assignment_id: Option<Uuid>,
    pub handoff_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CreateAssignment {
    pub task_id: String,
    pub provider_plugin_id: Uuid,
    pub provider_account_id: Uuid,
    pub model_definition_id: Option<Uuid>,
    pub runtime_profile_id: Uuid,
    pub agent_profile_id: Uuid,
    pub permission_policy_id: Option<Uuid>,
    pub required_capabilities: Vec<String>,
    pub assigned_by: Option<String>,
    pub handoff_from_assignment_id: Option<Uuid>,
    pub handoff_reason: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ProviderSession {
    pub id: Uuid,
    pub assignment_id: Uuid,
    pub provider_session_reference: Option<String>,
    pub provider_thread_reference: Option<String>,
    pub external_session_reference: Option<String>,
    pub resume_strategy: String,
    pub mode: String,
    pub status: String,
    pub context_package_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    #[ts(type = "unknown")]
    pub provider_metadata_json: Json<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ProviderSessionEvent {
    pub id: Uuid,
    pub session_id: Uuid,
    pub event_type: String,
    #[ts(type = "unknown")]
    pub payload_json: Json<Value>,
    pub created_at: DateTime<Utc>,
}

macro_rules! list_all {
    ($name:ident, $table:literal, $order:literal) => {
        impl $name {
            pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
                sqlx::query_as::<_, Self>(concat!("SELECT * FROM ", $table, " ORDER BY ", $order))
                    .fetch_all(pool)
                    .await
            }
        }
    };
}

list_all!(ProviderPlugin, "provider_plugins", "display_name ASC");
list_all!(ProviderAccount, "provider_accounts", "display_name ASC");
list_all!(ModelDefinition, "model_definitions", "display_name ASC");
list_all!(RuntimeProfile, "runtime_profiles", "name ASC");
list_all!(ProviderPermissionPolicy, "permission_policies", "name ASC");
list_all!(AgentProfile, "agent_profiles", "name ASC");
list_all!(Assignment, "assignments", "assigned_at DESC");
list_all!(ProviderSession, "provider_sessions", "started_at DESC");

impl ProviderPlugin {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM provider_plugins WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: CreateProviderPlugin,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO provider_plugins (id, plugin_key, display_name, version, vendor, description, manifest_path, adapter_entrypoint, configuration_schema, credential_schema, capabilities, runtime_types, permissions) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(data.plugin_key)
        .bind(data.display_name)
        .bind(data.version)
        .bind(data.vendor)
        .bind(data.description)
        .bind(data.manifest_path)
        .bind(data.adapter_entrypoint)
        .bind(Json(data.configuration_schema))
        .bind(Json(data.credential_schema))
        .bind(Json(data.capabilities))
        .bind(Json(data.runtime_types))
        .bind(Json(data.permissions))
        .execute(pool)
        .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl ProviderAccount {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM provider_accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: CreateProviderAccount,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO provider_accounts (id, provider_plugin_id, display_name, account_type, credential_reference, configuration_json) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(data.provider_plugin_id)
            .bind(data.display_name)
            .bind(data.account_type)
            .bind(data.credential_reference)
            .bind(Json(data.configuration_json))
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl ModelDefinition {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM model_definitions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: CreateModelDefinition,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO model_definitions (id, provider_account_id, provider_model_key, display_name, description, context_window, input_modalities, output_modalities, capability_json, pricing_json, discovered_automatically) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(data.provider_account_id)
            .bind(data.provider_model_key)
            .bind(data.display_name)
            .bind(data.description)
            .bind(data.context_window)
            .bind(Json(data.input_modalities))
            .bind(Json(data.output_modalities))
            .bind(Json(data.capabilities))
            .bind(Json(data.pricing_json))
            .bind(data.discovered_automatically)
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl RuntimeProfile {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM runtime_profiles WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: CreateRuntimeProfile,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO runtime_profiles (id, provider_account_id, name, runtime_type, executable_path, endpoint, remote_connection_id, working_directory_policy, environment_reference, configuration_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(data.provider_account_id)
            .bind(data.name)
            .bind(data.runtime_type)
            .bind(data.executable_path)
            .bind(data.endpoint)
            .bind(data.remote_connection_id)
            .bind(data.working_directory_policy)
            .bind(data.environment_reference)
            .bind(Json(data.configuration_json))
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl ProviderPermissionPolicy {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM permission_policies WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: CreateProviderPermissionPolicy,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO permission_policies (id, name, description, approved_permissions, constraints_json) VALUES (?, ?, ?, ?, ?)")
            .bind(id)
            .bind(data.name)
            .bind(data.description)
            .bind(Json(data.approved_permissions))
            .bind(Json(data.constraints_json))
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl AgentProfile {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM agent_profiles WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(pool: &SqlitePool, data: CreateAgentProfile) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_profiles (id, name, description, role_key, system_instructions, compatible_capabilities, preferred_provider_plugin_id, preferred_model_id, allowed_tools, denied_tools, permission_policy_id, context_policy_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(data.name)
            .bind(data.description)
            .bind(data.role_key)
            .bind(data.system_instructions)
            .bind(Json(data.compatible_capabilities))
            .bind(data.preferred_provider_plugin_id)
            .bind(data.preferred_model_id)
            .bind(Json(data.allowed_tools))
            .bind(Json(data.denied_tools))
            .bind(data.permission_policy_id)
            .bind(Json(data.context_policy_json))
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl Assignment {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM assignments WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(pool: &SqlitePool, data: CreateAssignment) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO assignments (id, task_id, provider_plugin_id, provider_account_id, model_definition_id, runtime_profile_id, agent_profile_id, permission_policy_id, required_capabilities, assigned_by, handoff_from_assignment_id, handoff_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(data.task_id)
            .bind(data.provider_plugin_id)
            .bind(data.provider_account_id)
            .bind(data.model_definition_id)
            .bind(data.runtime_profile_id)
            .bind(data.agent_profile_id)
            .bind(data.permission_policy_id)
            .bind(Json(data.required_capabilities))
            .bind(data.assigned_by.unwrap_or_else(|| "user".into()))
            .bind(data.handoff_from_assignment_id)
            .bind(data.handoff_reason)
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl ProviderSession {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>("SELECT * FROM provider_sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn create(
        pool: &SqlitePool,
        assignment_id: Uuid,
        resume_strategy: &str,
        mode: &str,
        context_package_id: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO provider_sessions (id, assignment_id, resume_strategy, mode, context_package_id) VALUES (?, ?, ?, ?, ?)")
            .bind(id)
            .bind(assignment_id)
            .bind(resume_strategy)
            .bind(mode)
            .bind(context_package_id)
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn mark_started(
        pool: &SqlitePool,
        id: Uuid,
        provider_session_reference: Option<String>,
        provider_thread_reference: Option<String>,
        metadata: Value,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query("UPDATE provider_sessions SET status = 'running', provider_session_reference = ?, provider_thread_reference = ?, provider_metadata_json = ?, last_message_at = datetime('now', 'subsec') WHERE id = ?")
            .bind(provider_session_reference)
            .bind(provider_thread_reference)
            .bind(Json(metadata))
            .bind(id)
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn mark_failed(
        pool: &SqlitePool,
        id: Uuid,
        code: &str,
        message: &str,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query("UPDATE provider_sessions SET status = 'failed', error_code = ?, error_message = ?, ended_at = datetime('now', 'subsec') WHERE id = ?")
            .bind(code)
            .bind(message)
            .bind(id)
            .execute(pool)
            .await?;
        Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

impl ProviderSessionEvent {
    pub async fn create(
        pool: &SqlitePool,
        session_id: Uuid,
        event_type: &str,
        payload: Value,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO provider_session_events (id, session_id, event_type, payload_json) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(session_id)
            .bind(event_type)
            .bind(Json(payload))
            .execute(pool)
            .await?;
        sqlx::query_as::<_, Self>("SELECT * FROM provider_session_events WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
    }
}
