use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use ts_rs::TS;

pub const MANIFEST_FILE_NAME: &str = "provider-plugin.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPluginManifest {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub entrypoint: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub configuration_schema: Option<Value>,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub credential_schema: Option<Value>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub runtime_types: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestValidationError {
    #[error("unsupported manifest schema version: {0}")]
    UnsupportedSchemaVersion(String),
    #[error("plugin id must be a reverse-DNS style identifier")]
    InvalidPluginId,
    #[error("plugin name must not be empty")]
    EmptyName,
    #[error("plugin version must not be empty")]
    EmptyVersion,
    #[error("entrypoint must be a relative path inside the plugin directory")]
    UnsafeEntrypoint,
    #[error("capability id is invalid: {0}")]
    InvalidCapability(String),
    #[error("runtime type is invalid: {0}")]
    InvalidRuntimeType(String),
    #[error("permission id is invalid: {0}")]
    InvalidPermission(String),
    #[error("duplicate capability id: {0}")]
    DuplicateCapability(String),
    #[error("duplicate runtime type: {0}")]
    DuplicateRuntimeType(String),
    #[error("duplicate permission id: {0}")]
    DuplicatePermission(String),
}

impl ProviderPluginManifest {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        if self.schema_version != "1.0" {
            return Err(ManifestValidationError::UnsupportedSchemaVersion(
                self.schema_version.clone(),
            ));
        }
        if !is_reverse_dns_id(&self.id) {
            return Err(ManifestValidationError::InvalidPluginId);
        }
        if self.name.trim().is_empty() {
            return Err(ManifestValidationError::EmptyName);
        }
        if self.version.trim().is_empty() {
            return Err(ManifestValidationError::EmptyVersion);
        }

        let entrypoint = Path::new(&self.entrypoint);
        if self.entrypoint.trim().is_empty()
            || entrypoint.is_absolute()
            || entrypoint
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(ManifestValidationError::UnsafeEntrypoint);
        }

        validate_unique_ids(
            &self.capabilities,
            ManifestValidationError::InvalidCapability,
            ManifestValidationError::DuplicateCapability,
        )?;
        validate_unique_ids(
            &self.runtime_types,
            ManifestValidationError::InvalidRuntimeType,
            ManifestValidationError::DuplicateRuntimeType,
        )?;
        validate_unique_ids(
            &self.permissions,
            ManifestValidationError::InvalidPermission,
            ManifestValidationError::DuplicatePermission,
        )?;
        Ok(())
    }
}

fn is_reverse_dns_id(value: &str) -> bool {
    let segments: Vec<_> = value.split('.').collect();
    segments.len() >= 3
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        })
}

fn is_extension_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn validate_unique_ids(
    values: &[String],
    invalid: impl Fn(String) -> ManifestValidationError,
    duplicate: impl Fn(String) -> ManifestValidationError,
) -> Result<(), ManifestValidationError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !is_extension_id(value) {
            return Err(invalid(value.clone()));
        }
        if !seen.insert(value) {
            return Err(duplicate(value.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> ProviderPluginManifest {
        ProviderPluginManifest {
            schema_version: "1.0".into(),
            id: "dev.taskai.mock".into(),
            name: "Mock Provider".into(),
            version: "1.0.0".into(),
            vendor: None,
            description: None,
            entrypoint: "bin/mock-provider".into(),
            configuration_schema: None,
            credential_schema: None,
            capabilities: vec!["chat".into(), "session_resume".into()],
            runtime_types: vec!["local_runtime".into()],
            permissions: vec!["process_execution".into()],
        }
    }

    #[test]
    fn accepts_provider_independent_manifest() {
        assert_eq!(valid_manifest().validate(), Ok(()));
    }

    #[test]
    fn rejects_parent_directory_entrypoint() {
        let mut manifest = valid_manifest();
        manifest.entrypoint = "../outside".into();
        assert_eq!(
            manifest.validate(),
            Err(ManifestValidationError::UnsafeEntrypoint)
        );
    }

    #[test]
    fn rejects_duplicate_capability() {
        let mut manifest = valid_manifest();
        manifest.capabilities.push("chat".into());
        assert_eq!(
            manifest.validate(),
            Err(ManifestValidationError::DuplicateCapability("chat".into()))
        );
    }
}
