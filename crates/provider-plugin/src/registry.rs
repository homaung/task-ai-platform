use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{MANIFEST_FILE_NAME, ManifestValidationError, ProviderPluginManifest};

#[derive(Debug, Clone)]
pub struct DiscoveredProviderPlugin {
    pub manifest: ProviderPluginManifest,
    pub root_path: PathBuf,
    pub manifest_path: PathBuf,
    pub entrypoint_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum PluginRegistryError {
    #[error("failed to read plugin file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid plugin manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Validation(#[from] ManifestValidationError),
    #[error("plugin entrypoint does not exist: {0}")]
    EntrypointMissing(PathBuf),
    #[error("plugin entrypoint resolves outside its plugin directory")]
    EntrypointEscapesRoot,
}

#[derive(Debug, Clone)]
pub struct PluginRegistry {
    root: PathBuf,
}

impl PluginRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn discover(&self) -> Result<Vec<DiscoveredProviderPlugin>, PluginRegistryError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();
        let root_manifest = self.root.join(MANIFEST_FILE_NAME);
        if root_manifest.is_file() {
            plugins.push(Self::load_manifest(&root_manifest)?);
        }

        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let manifest_path = entry.path().join(MANIFEST_FILE_NAME);
            if manifest_path.is_file() {
                plugins.push(Self::load_manifest(&manifest_path)?);
            }
        }

        plugins.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(plugins)
    }

    pub fn load_directory(
        plugin_directory: impl AsRef<Path>,
    ) -> Result<DiscoveredProviderPlugin, PluginRegistryError> {
        Self::load_manifest(&plugin_directory.as_ref().join(MANIFEST_FILE_NAME))
    }

    pub fn load_manifest(
        manifest_path: impl AsRef<Path>,
    ) -> Result<DiscoveredProviderPlugin, PluginRegistryError> {
        let manifest_path = manifest_path.as_ref();
        let manifest: ProviderPluginManifest =
            serde_json::from_str(&fs::read_to_string(manifest_path)?)?;
        manifest.validate()?;

        let root_path = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()?;
        let requested_entrypoint = root_path.join(&manifest.entrypoint);
        if !requested_entrypoint.is_file() {
            return Err(PluginRegistryError::EntrypointMissing(requested_entrypoint));
        }
        let entrypoint_path = requested_entrypoint.canonicalize()?;
        if !entrypoint_path.starts_with(&root_path) {
            return Err(PluginRegistryError::EntrypointEscapesRoot);
        }

        Ok(DiscoveredProviderPlugin {
            manifest,
            root_path,
            manifest_path: manifest_path.canonicalize()?,
            entrypoint_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn discovers_plugins_without_provider_name_switches() {
        let root = tempdir().unwrap();
        let plugin = root.path().join("custom");
        fs::create_dir_all(plugin.join("bin")).unwrap();
        fs::write(plugin.join("bin/agent"), "executable fixture").unwrap();
        fs::write(
            plugin.join(MANIFEST_FILE_NAME),
            r#"{
                "schemaVersion":"1.0",
                "id":"dev.example.custom-agent",
                "name":"Custom Agent",
                "version":"1.0.0",
                "entrypoint":"bin/agent",
                "capabilities":["chat"],
                "runtimeTypes":["local_runtime"],
                "permissions":[]
            }"#,
        )
        .unwrap();

        let discovered = PluginRegistry::new(root.path()).discover().unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].manifest.id, "dev.example.custom-agent");
    }
}
