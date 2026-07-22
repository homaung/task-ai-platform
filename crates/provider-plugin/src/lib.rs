//! Provider-independent plugin protocol and isolated process host.
//!
//! The platform core only depends on this capability-oriented contract. It
//! intentionally contains no provider or model names.

mod host;
mod manifest;
mod registry;

pub use host::{
    PluginHost, PluginHostError, PluginHostPolicy, ProviderPluginRequest, ProviderPluginResponse,
    ProviderProtocolError,
};
pub use manifest::{MANIFEST_FILE_NAME, ManifestValidationError, ProviderPluginManifest};
pub use registry::{DiscoveredProviderPlugin, PluginRegistry, PluginRegistryError};
