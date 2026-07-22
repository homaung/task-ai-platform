use std::{collections::BTreeSet, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use db::models::provider_platform::{
    AgentProfile, Assignment, CreateAgentProfile, CreateAssignment, CreateModelDefinition,
    CreateProviderAccount, CreateProviderPermissionPolicy, CreateProviderPlugin,
    CreateRuntimeProfile, ModelDefinition, ProviderAccount, ProviderPermissionPolicy,
    ProviderPlugin, ProviderSession, ProviderSessionEvent, RuntimeProfile,
};
use deployment::Deployment;
use provider_plugin::{PluginHost, PluginHostPolicy, PluginRegistry, ProviderPluginRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Serialize, TS)]
pub struct ProviderPlatformSnapshot {
    pub plugins: Vec<ProviderPlugin>,
    pub accounts: Vec<ProviderAccount>,
    pub models: Vec<ModelDefinition>,
    pub runtimes: Vec<RuntimeProfile>,
    pub permission_policies: Vec<ProviderPermissionPolicy>,
    pub agent_profiles: Vec<AgentProfile>,
    pub assignments: Vec<Assignment>,
    pub sessions: Vec<ProviderSession>,
}

#[derive(Debug, Deserialize, TS)]
pub struct InstallProviderPluginRequest {
    pub directory: String,
}

#[derive(Debug, Clone, Serialize, TS)]
pub struct AssignmentValidation {
    pub valid: bool,
    pub available_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
    pub missing_permissions: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, TS)]
pub struct CreateAssignmentResponse {
    pub assignment: Assignment,
    pub validation: AssignmentValidation,
}

#[derive(Debug, Deserialize, TS)]
pub struct StartProviderSessionRequest {
    pub assignment_id: Uuid,
    #[serde(default)]
    pub context_package_id: Option<String>,
    #[serde(default = "default_session_mode")]
    pub mode: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub input: Value,
}

fn default_session_mode() -> String {
    "interactive".into()
}

#[derive(Debug, Serialize, TS)]
pub struct StartProviderSessionResponse {
    pub session: ProviderSession,
    #[ts(type = "unknown")]
    pub adapter_result: Option<Value>,
}

pub async fn snapshot(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<ProviderPlatformSnapshot>>, ApiError> {
    let pool = &deployment.db().pool;
    Ok(ResponseJson(ApiResponse::success(
        ProviderPlatformSnapshot {
            plugins: ProviderPlugin::find_all(pool).await?,
            accounts: ProviderAccount::find_all(pool).await?,
            models: ModelDefinition::find_all(pool).await?,
            runtimes: RuntimeProfile::find_all(pool).await?,
            permission_policies: ProviderPermissionPolicy::find_all(pool).await?,
            agent_profiles: AgentProfile::find_all(pool).await?,
            assignments: Assignment::find_all(pool).await?,
            sessions: ProviderSession::find_all(pool).await?,
        },
    )))
}

pub async fn install_plugin(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<InstallProviderPluginRequest>,
) -> Result<ResponseJson<ApiResponse<ProviderPlugin>>, ApiError> {
    let discovered = PluginRegistry::load_directory(&payload.directory)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let manifest = discovered.manifest;
    let plugin = ProviderPlugin::create(
        &deployment.db().pool,
        CreateProviderPlugin {
            plugin_key: manifest.id,
            display_name: manifest.name,
            version: manifest.version,
            vendor: manifest.vendor,
            description: manifest.description,
            manifest_path: discovered.manifest_path.to_string_lossy().into_owned(),
            adapter_entrypoint: discovered.entrypoint_path.to_string_lossy().into_owned(),
            configuration_schema: manifest.configuration_schema.unwrap_or_else(|| json!({})),
            credential_schema: manifest.credential_schema.unwrap_or_else(|| json!({})),
            capabilities: manifest.capabilities,
            runtime_types: manifest.runtime_types,
            permissions: manifest.permissions,
        },
    )
    .await?;
    Ok(ResponseJson(ApiResponse::success(plugin)))
}

pub async fn create_account(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateProviderAccount>,
) -> Result<ResponseJson<ApiResponse<ProviderAccount>>, ApiError> {
    ensure_plugin(&deployment, payload.provider_plugin_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        ProviderAccount::create(&deployment.db().pool, payload).await?,
    )))
}

pub async fn create_model(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateModelDefinition>,
) -> Result<ResponseJson<ApiResponse<ModelDefinition>>, ApiError> {
    ensure_account(&deployment, payload.provider_account_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        ModelDefinition::create(&deployment.db().pool, payload).await?,
    )))
}

pub async fn create_runtime(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateRuntimeProfile>,
) -> Result<ResponseJson<ApiResponse<RuntimeProfile>>, ApiError> {
    ensure_account(&deployment, payload.provider_account_id).await?;
    Ok(ResponseJson(ApiResponse::success(
        RuntimeProfile::create(&deployment.db().pool, payload).await?,
    )))
}

pub async fn create_permission_policy(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateProviderPermissionPolicy>,
) -> Result<ResponseJson<ApiResponse<ProviderPermissionPolicy>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        ProviderPermissionPolicy::create(&deployment.db().pool, payload).await?,
    )))
}

pub async fn create_agent_profile(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateAgentProfile>,
) -> Result<ResponseJson<ApiResponse<AgentProfile>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        AgentProfile::create(&deployment.db().pool, payload).await?,
    )))
}

pub async fn validate_assignment(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateAssignment>,
) -> Result<ResponseJson<ApiResponse<AssignmentValidation>>, ApiError> {
    let validation = validate_assignment_input(&deployment, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(validation)))
}

pub async fn create_assignment(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateAssignment>,
) -> Result<ResponseJson<ApiResponse<CreateAssignmentResponse>>, ApiError> {
    let validation = validate_assignment_input(&deployment, &payload).await?;
    if !validation.valid {
        return Err(ApiError::BadRequest(format!(
            "assignment does not satisfy task requirements; missing capabilities: {:?}; missing permissions: {:?}; warnings: {:?}",
            validation.missing_capabilities, validation.missing_permissions, validation.warnings
        )));
    }
    let assignment = Assignment::create(&deployment.db().pool, payload).await?;
    Ok(ResponseJson(ApiResponse::success(
        CreateAssignmentResponse {
            assignment,
            validation,
        },
    )))
}

pub async fn start_session(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<StartProviderSessionRequest>,
) -> Result<ResponseJson<ApiResponse<StartProviderSessionResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let assignment = Assignment::find_by_id(pool, payload.assignment_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("assignment not found".into()))?;
    let plugin = ensure_plugin(&deployment, assignment.provider_plugin_id).await?;
    let account = ensure_account(&deployment, assignment.provider_account_id).await?;
    let runtime = RuntimeProfile::find_by_id(pool, assignment.runtime_profile_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("runtime profile not found".into()))?;
    let agent = AgentProfile::find_by_id(pool, assignment.agent_profile_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("agent profile not found".into()))?;
    let model = match assignment.model_definition_id {
        Some(id) => ModelDefinition::find_by_id(pool, id).await?,
        None => None,
    };
    let policy = match assignment.permission_policy_id {
        Some(id) => ProviderPermissionPolicy::find_by_id(pool, id).await?,
        None => None,
    };

    let resume_strategy = if plugin
        .capabilities
        .0
        .iter()
        .any(|value| value == "session_resume")
    {
        "resume_or_new"
    } else {
        "new_session_with_context"
    };
    let session = ProviderSession::create(
        pool,
        assignment.id,
        resume_strategy,
        &payload.mode,
        payload.context_package_id.clone(),
    )
    .await?;

    let discovered = PluginRegistry::load_manifest(&plugin.manifest_path)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let approved_permissions = policy
        .as_ref()
        .map(|policy| policy.approved_permissions.0.iter().cloned().collect())
        .unwrap_or_default();
    let host_policy = PluginHostPolicy {
        approved_permissions,
        timeout: Duration::from_secs(30),
        max_response_bytes: 1024 * 1024,
    };
    let request = ProviderPluginRequest {
        id: session.id.to_string(),
        method: "startSession".into(),
        params: json!({
            "assignment": assignment,
            "account": account,
            "model": model,
            "runtime": runtime,
            "agentProfile": agent,
            "contextPackageId": payload.context_package_id,
            "input": payload.input,
        }),
    };

    match PluginHost::call(&discovered, request, &host_policy).await {
        Ok(result) => {
            let provider_session_reference = string_field(
                &result,
                &["providerSessionReference", "provider_session_reference"],
            );
            let provider_thread_reference = string_field(
                &result,
                &["providerThreadReference", "provider_thread_reference"],
            );
            let metadata = result.get("metadata").cloned().unwrap_or_else(|| json!({}));
            let session = ProviderSession::mark_started(
                pool,
                session.id,
                provider_session_reference,
                provider_thread_reference,
                metadata,
            )
            .await?;
            ProviderSessionEvent::create(pool, session.id, "session_started", result.clone())
                .await?;
            Ok(ResponseJson(ApiResponse::success(
                StartProviderSessionResponse {
                    session,
                    adapter_result: Some(result),
                },
            )))
        }
        Err(error) => {
            let session = ProviderSession::mark_failed(
                pool,
                session.id,
                "plugin_host_error",
                &error.to_string(),
            )
            .await?;
            ProviderSessionEvent::create(
                pool,
                session.id,
                "session_failed",
                json!({ "error": error.to_string() }),
            )
            .await?;
            Ok(ResponseJson(ApiResponse::success(
                StartProviderSessionResponse {
                    session,
                    adapter_result: None,
                },
            )))
        }
    }
}

async fn validate_assignment_input(
    deployment: &DeploymentImpl,
    payload: &CreateAssignment,
) -> Result<AssignmentValidation, ApiError> {
    let pool = &deployment.db().pool;
    let plugin = ensure_plugin(deployment, payload.provider_plugin_id).await?;
    let account = ensure_account(deployment, payload.provider_account_id).await?;
    let runtime = RuntimeProfile::find_by_id(pool, payload.runtime_profile_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("runtime profile not found".into()))?;
    let agent = AgentProfile::find_by_id(pool, payload.agent_profile_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("agent profile not found".into()))?;
    let model = match payload.model_definition_id {
        Some(id) => Some(
            ModelDefinition::find_by_id(pool, id)
                .await?
                .ok_or_else(|| ApiError::BadRequest("model definition not found".into()))?,
        ),
        None => None,
    };
    let policy = match payload.permission_policy_id {
        Some(id) => Some(
            ProviderPermissionPolicy::find_by_id(pool, id)
                .await?
                .ok_or_else(|| ApiError::BadRequest("permission policy not found".into()))?,
        ),
        None => None,
    };

    let mut warnings = Vec::new();
    if account.provider_plugin_id != plugin.id {
        warnings.push("selected account does not belong to the selected plugin".into());
    }
    if runtime.provider_account_id != account.id {
        warnings.push("selected runtime does not belong to the selected account".into());
    }
    if !plugin.runtime_types.0.contains(&runtime.runtime_type) {
        warnings.push(format!(
            "runtime type '{}' is not declared by the plugin",
            runtime.runtime_type
        ));
    }
    if let Some(model) = &model
        && model.provider_account_id != account.id
    {
        warnings.push("selected model does not belong to the selected account".into());
    }
    if !plugin.enabled || !account.enabled || !runtime.enabled || !agent.enabled {
        warnings.push("one or more selected resources are disabled".into());
    }

    let available = model
        .as_ref()
        .filter(|model| !model.capability_json.0.is_empty())
        .map(|model| model.capability_json.0.clone())
        .unwrap_or_else(|| plugin.capabilities.0.clone());

    let approved_permissions = policy
        .as_ref()
        .map(|policy| policy.approved_permissions.0.clone())
        .unwrap_or_default();
    Ok(evaluate_requirements(
        &available,
        &payload.required_capabilities,
        &plugin.permissions.0,
        &approved_permissions,
        warnings,
    ))
}

fn evaluate_requirements(
    available_capabilities: &[String],
    required_capabilities: &[String],
    required_permissions: &[String],
    approved_permissions: &[String],
    warnings: Vec<String>,
) -> AssignmentValidation {
    let available: BTreeSet<_> = available_capabilities.iter().cloned().collect();
    let required: BTreeSet<_> = required_capabilities.iter().cloned().collect();
    let approved: BTreeSet<_> = approved_permissions.iter().cloned().collect();
    let permissions: BTreeSet<_> = required_permissions.iter().cloned().collect();
    let missing_capabilities = required.difference(&available).cloned().collect::<Vec<_>>();
    let missing_permissions = permissions
        .difference(&approved)
        .cloned()
        .collect::<Vec<_>>();
    AssignmentValidation {
        valid: missing_capabilities.is_empty()
            && missing_permissions.is_empty()
            && warnings.is_empty(),
        available_capabilities: available.into_iter().collect(),
        missing_capabilities,
        missing_permissions,
        warnings,
    }
}

async fn ensure_plugin(deployment: &DeploymentImpl, id: Uuid) -> Result<ProviderPlugin, ApiError> {
    ProviderPlugin::find_by_id(&deployment.db().pool, id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("provider plugin not found".into()))
}

async fn ensure_account(
    deployment: &DeploymentImpl,
    id: Uuid,
) -> Result<ProviderAccount, ApiError> {
    ProviderAccount::find_by_id(&deployment.db().pool, id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("provider account not found".into()))
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_str).map(str::to_owned))
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/provider-platform", get(snapshot))
        .route("/provider-platform/plugins/install", post(install_plugin))
        .route("/provider-platform/accounts", post(create_account))
        .route("/provider-platform/models", post(create_model))
        .route("/provider-platform/runtimes", post(create_runtime))
        .route(
            "/provider-platform/permission-policies",
            post(create_permission_policy),
        )
        .route(
            "/provider-platform/agent-profiles",
            post(create_agent_profile),
        )
        .route(
            "/provider-platform/assignments/validate",
            post(validate_assignment),
        )
        .route("/provider-platform/assignments", post(create_assignment))
        .route("/provider-platform/sessions/start", post(start_session))
        .route(
            "/provider-platform/plugins/{plugin_id}",
            get(
                |State(deployment): State<DeploymentImpl>, Path(plugin_id): Path<Uuid>| async move {
                    let plugin = ensure_plugin(&deployment, plugin_id).await?;
                    Ok::<_, ApiError>(ResponseJson(ApiResponse::<ProviderPlugin>::success(plugin)))
                },
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_task_capabilities() {
        let validation = evaluate_requirements(
            &["filesystem_read".into()],
            &[
                "filesystem_read".into(),
                "filesystem_write".into(),
                "command_execution".into(),
            ],
            &[],
            &[],
            vec![],
        );
        assert!(!validation.valid);
        assert_eq!(
            validation.missing_capabilities,
            vec!["command_execution", "filesystem_write"]
        );
    }

    #[test]
    fn requires_explicit_plugin_permission_approval() {
        let validation = evaluate_requirements(
            &["chat".into()],
            &["chat".into()],
            &["network_access".into(), "process_execution".into()],
            &["network_access".into()],
            vec![],
        );
        assert!(!validation.valid);
        assert_eq!(validation.missing_permissions, vec!["process_execution"]);
    }
}
