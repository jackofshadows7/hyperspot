use serde::de::DeserializeOwned;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Provider of module-specific configuration (raw JSON sections only).
pub trait ConfigProvider: Send + Sync {
    /// Returns raw JSON section for the module, if any.
    fn get_module_config(&self, module_name: &str) -> Option<&serde_json::Value>;
}

#[derive(Clone)]
pub struct ModuleCtx {
    pub(crate) db: Option<Arc<db::DbHandle>>,
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
    pub fn with_db(mut self, db: Arc<db::DbHandle>) -> Self {
        self.inner.db = Some(db);
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
    pub fn db(&self) -> Option<Arc<db::DbHandle>> {
        self.db.clone()
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

    /// Best-effort: deserialize the module's config into `T`, fallback to `T::default()`
    /// if section is missing or invalid.
    pub fn module_config<T: DeserializeOwned + Default>(&self) -> T {
        match (&self.module_name, &self.config_provider) {
            (Some(name), Some(p)) => p
                .get_module_config(name)
                .and_then(|v| serde_json::from_value::<T>(v.clone()).ok())
                .unwrap_or_default(),
            _ => T::default(),
        }
    }

    /// Strict: deserialize the module's config into `T`, returning a pathful error on failure.
    pub fn module_config_required<T: serde::de::DeserializeOwned>(&self) -> anyhow::Result<T> {
        let name = self
            .module_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("context is not scoped to a module"))?;

        let prov = self
            .config_provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no ConfigProvider"))?;

        let val = prov
            .get_module_config(name)
            .ok_or_else(|| anyhow::anyhow!("missing module config: {name}"))?;

        let out: T = serde_json::from_value(val.clone())
            .map_err(|e| anyhow::anyhow!("invalid {name} config: {}", e))?;
        Ok(out)
    }
}
