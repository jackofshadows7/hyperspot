use serde::de::DeserializeOwned;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// Note: runtime-dependent features are conditionally compiled

/// Configuration error for typed config operations
#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("module '{module}' not found")]
    ModuleNotFound { module: String },
    #[error("module '{module}' config must be an object")]
    InvalidModuleStructure { module: String },
    #[error("missing 'config' section in module '{module}'")]
    MissingConfigSection { module: String },
    #[error("invalid config for module '{module}': {source}")]
    InvalidConfig {
        module: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Provider of module-specific configuration (raw JSON sections only).
pub trait ConfigProvider: Send + Sync {
    /// Returns raw JSON section for the module, if any.
    fn get_module_config(&self, module_name: &str) -> Option<&serde_json::Value>;
}

/// Extension trait for typed configuration access.
///
/// This trait provides the **recommended and documented** way for modules to request their configuration.
/// It's separate from `ConfigProvider` to maintain object safety while providing generic functionality.
pub trait ConfigProviderExt: ConfigProvider {
    /// Deserialize a module's config section into a typed struct.
    ///
    /// Extracts the 'config' field from: `modules.<name> = { database: ..., config: ... }`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// #[derive(serde::Deserialize)]
    /// struct MyModuleConfig {
    ///     api_key: String,
    ///     timeout_ms: u64,
    /// }
    ///
    /// let config: MyModuleConfig = provider.module_config_typed("my_module")?;
    /// ```
    fn module_config_typed<T: DeserializeOwned>(
        &self,
        module_name: &str,
    ) -> Result<T, ConfigError> {
        let module_raw =
            self.get_module_config(module_name)
                .ok_or_else(|| ConfigError::ModuleNotFound {
                    module: module_name.to_string(),
                })?;

        // Extract config section from: modules.<name> = { database: ..., config: ... }
        let obj = module_raw
            .as_object()
            .ok_or_else(|| ConfigError::InvalidModuleStructure {
                module: module_name.to_string(),
            })?;

        let config_section =
            obj.get("config")
                .ok_or_else(|| ConfigError::MissingConfigSection {
                    module: module_name.to_string(),
                })?;

        let config: T = serde_json::from_value(config_section.clone()).map_err(|e| {
            ConfigError::InvalidConfig {
                module: module_name.to_string(),
                source: e,
            }
        })?;

        Ok(config)
    }
}

// Blanket implementation for all ConfigProvider types
impl<T: ConfigProvider> ConfigProviderExt for T {}

/// Helper function to deserialize module config from a ConfigProvider
///
/// This function provides typed configuration access for `dyn ConfigProvider` objects.
pub fn module_config_typed<T: DeserializeOwned>(
    provider: &dyn ConfigProvider,
    module_name: &str,
) -> Result<T, ConfigError> {
    let module_raw =
        provider
            .get_module_config(module_name)
            .ok_or_else(|| ConfigError::ModuleNotFound {
                module: module_name.to_string(),
            })?;

    // Extract config section from: modules.<name> = { database: ..., config: ... }
    let obj = module_raw
        .as_object()
        .ok_or_else(|| ConfigError::InvalidModuleStructure {
            module: module_name.to_string(),
        })?;

    let config_section = obj
        .get("config")
        .ok_or_else(|| ConfigError::MissingConfigSection {
            module: module_name.to_string(),
        })?;

    let config: T =
        serde_json::from_value(config_section.clone()).map_err(|e| ConfigError::InvalidConfig {
            module: module_name.to_string(),
            source: e,
        })?;

    Ok(config)
}

#[derive(Clone)]
pub struct ModuleCtx {
    pub(crate) db: Option<Arc<modkit_db::DbHandle>>,
    pub(crate) db_manager: Option<Arc<modkit_db::DbManager>>,
    pub(crate) config_provider: Option<Arc<dyn ConfigProvider>>,
    pub(crate) client_hub: Arc<crate::client_hub::ClientHub>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) module_name: Option<Arc<str>>,
}

// ---- construction/scoping (crate-private) ----
// TODO: make it private (but need to fix test crate visibility issues)
pub struct ModuleCtxBuilder {
    inner: ModuleCtx,
}

impl ModuleCtxBuilder {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            inner: ModuleCtx::from_token(token),
        }
    }
    pub fn with_db(mut self, db: Arc<modkit_db::DbHandle>) -> Self {
        self.inner.db = Some(db);
        self
    }
    pub fn with_db_manager(mut self, db_manager: Arc<modkit_db::DbManager>) -> Self {
        self.inner.db_manager = Some(db_manager);
        self
    }
    pub fn with_config_provider(mut self, p: Arc<dyn ConfigProvider>) -> Self {
        self.inner.config_provider = Some(p);
        self
    }
    pub(crate) fn with_client_hub(mut self, hub: Arc<crate::client_hub::ClientHub>) -> Self {
        self.inner.client_hub = hub;
        self
    }
    pub fn build(self) -> ModuleCtx {
        self.inner
    }
}

impl ModuleCtx {
    pub(crate) fn from_token(token: CancellationToken) -> Self {
        Self {
            db: None,
            db_manager: None,
            config_provider: None,
            client_hub: Arc::new(crate::client_hub::ClientHub::default()),
            cancellation_token: token,
            module_name: None,
        }
    }

    /// Scope context to a specific module name (used by the registry).
    pub(crate) fn for_module(mut self, name: &str) -> Self {
        self.module_name = Some(Arc::<str>::from(name));
        self
    }

    // ---- public read-only API for modules ----
    pub fn db(&self) -> Option<&modkit_db::DbHandle> {
        self.db.as_deref()
    }

    pub fn db_required(&self) -> modkit_db::Result<&modkit_db::DbHandle> {
        self.db.as_deref().ok_or_else(|| {
            modkit_db::DbError::FeatureDisabled("Database not configured for this module")
        })
    }

    /// Get a database handle for this module using the DbManager.
    /// Returns None if the module has no database configuration.
    pub async fn db_async(&self) -> anyhow::Result<Option<Arc<modkit_db::DbHandle>>> {
        match (&self.db_manager, &self.module_name) {
            (Some(manager), Some(module_name)) => {
                manager.get(module_name).await.map_err(anyhow::Error::from)
            }
            _ => Ok(None),
        }
    }

    /// Get a required database handle for this module using the DbManager.
    /// Returns an error if the module has no database configuration.
    pub async fn db_required_async(&self) -> anyhow::Result<Arc<modkit_db::DbHandle>> {
        let module_name = self
            .module_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Module name not set in context"))?;

        let manager = self
            .db_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Database manager not available"))?;

        manager
            .get(module_name)
            .await
            .map_err(anyhow::Error::from)?
            .ok_or_else(|| {
                anyhow::anyhow!("Database is not configured for module '{}'", module_name)
            })
    }

    pub fn client_hub(&self) -> Arc<crate::client_hub::ClientHub> {
        self.client_hub.clone()
    }

    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    pub fn current_module(&self) -> Option<&str> {
        self.module_name.as_deref()
    }

    /// Deserialize the module's config section into T.
    ///
    /// This method uses the new typed configuration system and provides better error messages.
    /// It extracts the 'config' field from: `modules.<name> = { database: ..., config: ... }`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// #[derive(serde::Deserialize)]
    /// struct MyConfig {
    ///     api_key: String,
    ///     timeout_ms: u64,
    /// }
    ///
    /// let config: MyConfig = ctx.config()?;
    /// ```
    pub fn config<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        let name = self
            .module_name
            .as_deref()
            .ok_or_else(|| ConfigError::ModuleNotFound {
                module: "unknown".to_string(),
            })?;

        let prov = self
            .config_provider
            .as_ref()
            .ok_or_else(|| ConfigError::ModuleNotFound {
                module: name.to_string(),
            })?;

        module_config_typed(prov.as_ref(), name)
    }

    /// Get the raw JSON value of the module's config section.
    /// Returns the 'config' field from: modules.<name> = { database: ..., config: ... }
    pub fn raw_config(&self) -> &serde_json::Value {
        use std::sync::LazyLock;

        static EMPTY: LazyLock<serde_json::Value> =
            LazyLock::new(|| serde_json::Value::Object(serde_json::Map::new()));

        match (&self.module_name, &self.config_provider) {
            (Some(name), Some(prov)) => {
                if let Some(module_raw) = prov.get_module_config(name) {
                    // Try new structure first: modules.<name> = { database: ..., config: ... }
                    if let Some(obj) = module_raw.as_object() {
                        if let Some(config_section) = obj.get("config") {
                            return config_section;
                        }
                    }
                }
                &EMPTY
            }
            _ => &EMPTY,
        }
    }

    /// Create a derivative context with the same references but a different DB handle.
    /// This allows reusing the stable base context while providing per-module DB access.
    pub fn with_db(&self, db: Arc<modkit_db::DbHandle>) -> ModuleCtx {
        ModuleCtx {
            db: Some(db),
            db_manager: self.db_manager.clone(),
            config_provider: self.config_provider.clone(),
            client_hub: self.client_hub.clone(),
            cancellation_token: self.cancellation_token.clone(),
            module_name: self.module_name.clone(),
        }
    }

    /// Create a derivative context with the same references but no DB handle.
    /// Useful for modules that don't require database access.
    pub fn without_db(&self) -> ModuleCtx {
        ModuleCtx {
            db: None,
            db_manager: self.db_manager.clone(),
            config_provider: self.config_provider.clone(),
            client_hub: self.client_hub.clone(),
            cancellation_token: self.cancellation_token.clone(),
            module_name: self.module_name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::collections::HashMap;

    #[derive(Debug, PartialEq, Deserialize)]
    struct TestConfig {
        api_key: String,
        timeout_ms: u64,
        enabled: bool,
    }

    struct MockConfigProvider {
        modules: HashMap<String, serde_json::Value>,
    }

    impl MockConfigProvider {
        fn new() -> Self {
            let mut modules = HashMap::new();

            // Valid module config
            modules.insert(
                "test_module".to_string(),
                json!({
                    "database": {
                        "url": "postgres://localhost/test"
                    },
                    "config": {
                        "api_key": "secret123",
                        "timeout_ms": 5000,
                        "enabled": true
                    }
                }),
            );

            // Module without config section
            modules.insert(
                "no_config_module".to_string(),
                json!({
                    "database": {
                        "url": "postgres://localhost/test"
                    }
                }),
            );

            // Module with invalid structure (not an object)
            modules.insert("invalid_module".to_string(), json!("not an object"));

            Self { modules }
        }
    }

    impl ConfigProvider for MockConfigProvider {
        fn get_module_config(&self, module_name: &str) -> Option<&serde_json::Value> {
            self.modules.get(module_name)
        }
    }

    #[test]
    fn test_module_config_typed_success() {
        let provider = MockConfigProvider::new();
        let result: Result<TestConfig, ConfigError> = provider.module_config_typed("test_module");

        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.api_key, "secret123");
        assert_eq!(config.timeout_ms, 5000);
        assert!(config.enabled);
    }

    #[test]
    fn test_module_config_typed_module_not_found() {
        let provider = MockConfigProvider::new();
        let result: Result<TestConfig, ConfigError> = provider.module_config_typed("nonexistent");

        assert!(matches!(result, Err(ConfigError::ModuleNotFound { .. })));
        if let Err(ConfigError::ModuleNotFound { module }) = result {
            assert_eq!(module, "nonexistent");
        }
    }

    #[test]
    fn test_module_config_typed_missing_config_section() {
        let provider = MockConfigProvider::new();
        let result: Result<TestConfig, ConfigError> =
            provider.module_config_typed("no_config_module");

        assert!(matches!(
            result,
            Err(ConfigError::MissingConfigSection { .. })
        ));
        if let Err(ConfigError::MissingConfigSection { module }) = result {
            assert_eq!(module, "no_config_module");
        }
    }

    #[test]
    fn test_module_config_typed_invalid_structure() {
        let provider = MockConfigProvider::new();
        let result: Result<TestConfig, ConfigError> =
            provider.module_config_typed("invalid_module");

        assert!(matches!(
            result,
            Err(ConfigError::InvalidModuleStructure { .. })
        ));
        if let Err(ConfigError::InvalidModuleStructure { module }) = result {
            assert_eq!(module, "invalid_module");
        }
    }

    #[test]
    fn test_module_config_typed_invalid_config() {
        let mut provider = MockConfigProvider::new();
        // Add module with invalid config structure
        provider.modules.insert(
            "bad_config_module".to_string(),
            json!({
                "config": {
                    "api_key": "secret123",
                    "timeout_ms": "not_a_number", // Should be u64
                    "enabled": true
                }
            }),
        );

        let result: Result<TestConfig, ConfigError> =
            provider.module_config_typed("bad_config_module");

        assert!(matches!(result, Err(ConfigError::InvalidConfig { .. })));
        if let Err(ConfigError::InvalidConfig { module, .. }) = result {
            assert_eq!(module, "bad_config_module");
        }
    }

    #[test]
    fn test_module_ctx_config() {
        let provider = Arc::new(MockConfigProvider::new());
        let ctx = ModuleCtxBuilder::new(CancellationToken::new())
            .with_config_provider(provider)
            .build()
            .for_module("test_module");

        let result: Result<TestConfig, ConfigError> = ctx.config();
        assert!(result.is_ok());

        let config = result.unwrap();
        assert_eq!(config.api_key, "secret123");
        assert_eq!(config.timeout_ms, 5000);
        assert!(config.enabled);
    }

    #[test]
    fn test_config_error_messages() {
        let module_not_found = ConfigError::ModuleNotFound {
            module: "test".to_string(),
        };
        assert_eq!(module_not_found.to_string(), "module 'test' not found");

        let invalid_structure = ConfigError::InvalidModuleStructure {
            module: "test".to_string(),
        };
        assert_eq!(
            invalid_structure.to_string(),
            "module 'test' config must be an object"
        );

        let missing_config = ConfigError::MissingConfigSection {
            module: "test".to_string(),
        };
        assert_eq!(
            missing_config.to_string(),
            "missing 'config' section in module 'test'"
        );
    }
}
