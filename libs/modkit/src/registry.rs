// modkit/src/registry/mod.rs
use anyhow::{anyhow, bail, Context, Result};
use axum::Router;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct ModuleEntry {
    pub name: &'static str,
    pub deps: &'static [&'static str],
    pub core: Arc<dyn crate::contracts::Module>,
    pub rest: Option<Arc<dyn crate::contracts::RestfulModule>>,
    pub rest_host: Option<Arc<dyn crate::contracts::RestHostModule>>,
    pub db: Option<Arc<dyn crate::contracts::DbModule>>,
    pub stateful: Option<Arc<dyn crate::contracts::StatefulModule>>,
}

/// The function type submitted by the macro via `inventory::submit!`.
/// NOTE: It now takes a *builder*, not the final registry.
pub struct Registrator(pub fn(&mut RegistryBuilder));

inventory::collect!(Registrator);

/// The final, topo-sorted runtime registry.
pub struct ModuleRegistry {
    modules: Vec<ModuleEntry>, // topo-sorted
}

impl ModuleRegistry {
    pub fn modules(&self) -> &[ModuleEntry] {
        &self.modules
    }

    /// Discover via inventory, have registrators fill the builder, then build & topo-sort.
    pub fn discover_and_build() -> Result<Self> {
        let mut b = RegistryBuilder::default();
        for r in ::inventory::iter::<Registrator> {
            (r.0)(&mut b);
        }
        b.build_topo_sorted()
    }

    // ---- Ordered phases: init → DB → REST (sync) → start → stop ----

    pub async fn run_init_phase(&self, base_ctx: &crate::context::ModuleCtx) -> Result<()> {
        for e in &self.modules {
            let ctx = base_ctx.clone().for_module(e.name);
            e.core
                .init(&ctx)
                .await
                .with_context(|| format!("Initialization failed for module '{}'", e.name))?;
        }
        Ok(())
    }

    pub async fn run_db_phase(&self, db: &db::DbHandle) -> Result<()> {
        for e in &self.modules {
            if let Some(dbm) = &e.db {
                // If you want advisory locks, do it here (kept minimal for portability):
                // let _lock = db.lock(e.name, "migration").await?;
                dbm.migrate(db)
                    .await
                    .with_context(|| format!("DB migration failed for module '{}'", e.name))?;
            }
        }
        Ok(())
    }

    pub fn run_rest_phase(
        &self,
        base_ctx: &crate::context::ModuleCtx,
        mut router: Router,
    ) -> Result<Router> {
        // Find host(s) and whether any rest modules exist
        let hosts: Vec<_> = self
            .modules
            .iter()
            .filter(|e| e.rest_host.is_some())
            .collect();
        match hosts.len() {
            0 => {
                if self.modules.iter().any(|e| e.rest.is_some()) {
                    anyhow::bail!(
                        "REST phase requires an ingress host: found modules with `capability \"rest\"` but no module with `capability \"rest_host\"`."
                    );
                } else {
                    return Ok(router);
                }
            }
            1 => { /* proceed */ }
            _ => anyhow::bail!("Multiple `rest_host` modules detected; exactly one is allowed."),
        }

        // Resolve the single host entry and its module context
        let host_idx = self
            .modules
            .iter()
            .position(|e| e.rest_host.is_some())
            .unwrap();
        let host_entry = &self.modules[host_idx];
        let host = host_entry.rest_host.as_ref().unwrap();
        let host_ctx = base_ctx.clone().for_module(host_entry.name);

        // use host as the registry
        let registry: &dyn crate::contracts::OpenApiRegistry = host.as_registry();

        // 1) Host prepare: base Router / global middlewares / basic OAS meta
        router = host.rest_prepare(&host_ctx, router)?;

        // 2) Register all REST providers (in the current discovery order;
        //    if you have a topo-ordered list, iterate that instead)
        for e in &self.modules {
            if let Some(rest) = &e.rest {
                let ctx = base_ctx.clone().for_module(e.name);
                router = rest
                    .register_rest(&ctx, router, registry)
                    .with_context(|| format!("REST registration failed for module '{}'", e.name))?;
            }
        }

        // 3) Host finalize: attach /openapi.json and /docs, persist Router if needed (no server start)
        router = host.rest_finalize(&host_ctx, router)?;
        Ok(router)
    }

    pub async fn run_start_phase(&self, cancel: CancellationToken) -> Result<()> {
        for e in &self.modules {
            if let Some(s) = &e.stateful {
                s.start(cancel.clone())
                    .await
                    .with_context(|| format!("Start failed for module '{}'", e.name))?;
            }
        }
        Ok(())
    }

    pub async fn run_stop_phase(&self, cancel: CancellationToken) -> Result<()> {
        for e in self.modules.iter().rev() {
            if let Some(s) = &e.stateful {
                if let Err(err) = s.stop(cancel.clone()).await {
                    tracing::warn!(module = e.name, error = %err, "Failed to stop module");
                }
            }
        }
        Ok(())
    }

    /// (Optional) quick lookup if you need it.
    pub fn get_module(&self, name: &str) -> Option<Arc<dyn crate::contracts::Module>> {
        self.modules
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.core.clone())
    }
}

/// Internal builder that macro registrators will feed.
/// Keys are module **names**; uniqueness enforced at build time.
#[derive(Default)]
pub struct RegistryBuilder {
    core: HashMap<&'static str, Arc<dyn crate::contracts::Module>>,
    deps: HashMap<&'static str, &'static [&'static str]>,
    rest: HashMap<&'static str, Arc<dyn crate::contracts::RestfulModule>>,
    rest_host: Option<(&'static str, Arc<dyn crate::contracts::RestHostModule>)>,
    db: HashMap<&'static str, Arc<dyn crate::contracts::DbModule>>,
    stateful: HashMap<&'static str, Arc<dyn crate::contracts::StatefulModule>>,
}

impl RegistryBuilder {
    pub fn register_core_with_meta(
        &mut self,
        name: &'static str,
        deps: &'static [&'static str],
        m: Arc<dyn crate::contracts::Module>,
    ) {
        if self.core.contains_key(name) {
            panic!("Module '{name}' is already registered");
        }
        self.core.insert(name, m);
        self.deps.insert(name, deps);
    }

    pub fn register_rest_with_meta(
        &mut self,
        name: &'static str,
        m: Arc<dyn crate::contracts::RestfulModule>,
    ) {
        self.rest.insert(name, m);
    }

    pub fn register_rest_host_with_meta(
        &mut self,
        name: &'static str,
        m: Arc<dyn crate::contracts::RestHostModule>,
    ) {
        if self.rest_host.is_some() {
            panic!("Multiple REST host modules detected: '{}' and '{}'. Only one REST host is allowed.", 
                   self.rest_host.as_ref().unwrap().0, name);
        }
        self.rest_host = Some((name, m));
    }

    pub fn register_db_with_meta(
        &mut self,
        name: &'static str,
        m: Arc<dyn crate::contracts::DbModule>,
    ) {
        self.db.insert(name, m);
    }

    pub fn register_stateful_with_meta(
        &mut self,
        name: &'static str,
        m: Arc<dyn crate::contracts::StatefulModule>,
    ) {
        self.stateful.insert(name, m);
    }

    /// Finalize & topo-sort; verify deps & capability binding to known cores.
    pub fn build_topo_sorted(self) -> Result<ModuleRegistry> {
        // 1) ensure every capability references a known core
        for (n, _) in self.rest.iter() {
            if !self.core.contains_key(n) {
                bail!("REST capability registered for unknown module '{}'", n);
            }
        }
        if let Some((n, _)) = &self.rest_host {
            if !self.core.contains_key(n) {
                bail!("REST host capability registered for unknown module '{}'", n);
            }
        }
        for (n, _) in self.db.iter() {
            if !self.core.contains_key(n) {
                bail!("DB capability registered for unknown module '{}'", n);
            }
        }
        for (n, _) in self.stateful.iter() {
            if !self.core.contains_key(n) {
                bail!("Stateful capability registered for unknown module '{}'", n);
            }
        }

        // 2) build graph over core modules
        let names: Vec<&'static str> = self.core.keys().copied().collect();
        let mut idx: HashMap<&'static str, usize> = HashMap::new();
        for (i, &n) in names.iter().enumerate() {
            idx.insert(n, i);
        }

        let mut indeg = vec![0usize; names.len()];
        let mut adj = vec![Vec::<usize>::new(); names.len()];

        for (&n, &deps) in self.deps.iter() {
            let &u = idx
                .get(n)
                .ok_or_else(|| anyhow!("Unknown module '{}'", n))?;
            for &d in deps {
                let &v = idx
                    .get(d)
                    .ok_or_else(|| anyhow!("Module '{}' depends on unknown '{}'", n, d))?;
                // edge d -> n (dep before module)
                adj[v].push(u);
                indeg[u] += 1;
            }
        }

        // 3) Kahn’s algorithm
        let mut q = VecDeque::new();
        for i in 0..names.len() {
            if indeg[i] == 0 {
                q.push_back(i);
            }
        }

        let mut order = Vec::with_capacity(names.len());
        while let Some(u) = q.pop_front() {
            order.push(u);
            for &w in &adj[u] {
                indeg[w] -= 1;
                if indeg[w] == 0 {
                    q.push_back(w);
                }
            }
        }
        if order.len() != names.len() {
            bail!("Cyclic dependency detected among modules");
        }

        // 4) Build final entries in topo order
        let mut entries = Vec::with_capacity(order.len());
        for i in order {
            let name = names[i];
            let deps = *self
                .deps
                .get(name)
                .ok_or_else(|| anyhow!("missing deps for '{}'", name))?;

            let core = self
                .core
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow!("core not found for '{}'", name))?;

            let entry = ModuleEntry {
                name,
                deps,
                core,
                rest: self.rest.get(name).cloned(),
                rest_host: self
                    .rest_host
                    .as_ref()
                    .filter(|(host_name, _)| *host_name == name)
                    .map(|(_, module)| module.clone()),
                db: self.db.get(name).cloned(),
                stateful: self.stateful.get(name).cloned(),
            };
            entries.push(entry);
        }

        tracing::info!(modules = ?entries.iter().map(|e| e.name).collect::<Vec<_>>(),
            "Module dependency order resolved (topo)");

        Ok(ModuleRegistry { modules: entries })
    }
}
