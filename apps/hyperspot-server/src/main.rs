use anyhow::Result;
use clap::{Parser, Subcommand};
use db::{ConnectOpts, DbHandle};
use mimalloc::MiMalloc;
use runtime::{AppConfig, AppConfigProvider, CliArgs, ConfigProvider};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// Adapter to make AppConfigProvider implement modkit::ConfigProvider
struct ModkitConfigAdapter(std::sync::Arc<AppConfigProvider>);

impl modkit::ConfigProvider for ModkitConfigAdapter {
    fn get_module_config(&self, module_name: &str) -> Option<&serde_json::Value> {
        self.0.get_module_config(module_name)
    }
}

// Per-module database factory implementation
fn create_per_module_db_factory(config: Arc<AppConfig>, home_dir: PathBuf) -> PerModuleDbFactory {
    Box::new(move |module_name: &str| {
        let config = config.clone();
        let home_dir = home_dir.clone();
        let module_name = module_name.to_string();

        Box::pin(async move {
            match runtime::config::build_final_db_for_module(&config, &module_name, &home_dir)? {
                Some((final_dsn, pool_cfg)) => {
                    // Convert from runtime config types to db config types
                    let connect_opts = db::ConnectOpts {
                        max_conns: pool_cfg.max_conns,
                        acquire_timeout: pool_cfg.acquire_timeout,
                        create_sqlite_dirs: true,
                        ..Default::default()
                    };

                    let redacted_dsn = redact_dsn_password(&final_dsn);
                    tracing::info!(
                        "Connecting to database for module '{}': {}",
                        module_name,
                        redacted_dsn
                    );

                    let db_handle = db::DbHandle::connect(&final_dsn, connect_opts)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to connect DB for module '{}': {}",
                                module_name,
                                e
                            )
                        })?;

                    Ok(Some(Arc::new(db_handle)))
                }
                None => {
                    // Module has no database configuration - this is fine
                    tracing::debug!("Module '{}' has no database configuration", module_name);
                    Ok(None)
                }
            }
        })
    })
}

/// Redact password from DSN for logging
fn redact_dsn_password(dsn: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(dsn) {
        if parsed.password().is_some() {
            let _ = parsed.set_password(Some("***"));
            parsed.to_string()
        } else {
            dsn.to_string()
        }
    } else {
        dsn.to_string()
    }
}

use modkit::runtime::{run, DbFactory, DbOptions, PerModuleDbFactory, RunOptions, ShutdownOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// Ensure modules are linked and registered via inventory
#[allow(dead_code)]
fn _ensure_modules_linked() {
    // Make sure all modules are linked
    let _ = std::any::type_name::<api_ingress::ApiIngress>();
    #[cfg(feature = "users-info-example")]
    let _ = std::any::type_name::<users_info::UsersInfo>();
}

// Force SQLx driver registration for Any driver (workaround for SQLx 0.8)
#[allow(unused_imports)]
use sqlx::{postgres::Postgres, sqlite::Sqlite};

#[allow(dead_code)]
fn _ensure_drivers_linked() {
    // Make sure database drivers are linked for sqlx::any
    let _ = std::any::type_name::<Sqlite>();
    let _ = std::any::type_name::<Postgres>();
}

/// HyperSpot Server - modular platform for AI services
#[derive(Parser)]
#[command(name = "hyperspot-server")]
#[command(about = "HyperSpot Server - modular platform for AI services")]
#[command(version = "0.1.0")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Port for HTTP server (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Print current configuration and exit
    #[arg(long)]
    print_config: bool,

    /// Log verbosity level (-v info, -vv debug, -vvv trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Use mock database
    #[arg(long)]
    mock: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server
    Run,
    /// Check configuration
    Check,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Link SQLx drivers (Any driver quirk in 0.8)
    _ensure_drivers_linked();

    let cli = Cli::parse();

    // CLI args passed down to config/app
    let args = CliArgs {
        config: cli.config.as_ref().map(|p| p.to_string_lossy().to_string()),
        port: cli.port,
        print_config: cli.print_config,
        verbose: cli.verbose,
        mock: cli.mock,
    };

    // Load configuration (normalized home_dir is applied inside)
    let mut config = AppConfig::load_or_default(cli.config.as_deref())?;

    // Apply CLI overrides (port / verbosity)
    config.apply_cli_overrides(&args);

    // Initialize logging
    let logging_config = config.logging.as_ref().cloned().unwrap_or_default();
    runtime::logging::init_logging_from_config(&logging_config, Path::new(&config.server.home_dir));
    tracing::info!("HyperSpot Server starting");
    println!("Effective configuration:\n{:#?}", config.server);

    // Print config and exit if requested
    if cli.print_config {
        println!("{}", config.to_yaml()?);
        return Ok(());
    }

    // Execute command
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_server(config, args).await,
        Commands::Check => check_config(config).await,
    }
}

async fn run_server(config: AppConfig, args: CliArgs) -> Result<()> {
    tracing::info!("Initializing modules...");

    // Provide module configs to modkit
    let config_provider = Arc::new(ModkitConfigAdapter(Arc::new(AppConfigProvider::new(
        config.clone(),
    ))));

    // Base dir for resolving relative sqlite paths (already absolute & created)
    let base_dir = PathBuf::from(&config.server.home_dir);

    // Prepare DB options - use new per-module system if database config is present
    let db_options = if config.database.is_some() {
        if args.mock {
            // For mock mode, still use the legacy system with in-memory SQLite
            tracing::info!("Mock mode: using in-memory SQLite for all modules");
            let factory: DbFactory = Box::new(move || {
                Box::pin(async move {
                    let connect_opts = ConnectOpts {
                        max_conns: Some(10),
                        acquire_timeout: Some(Duration::from_secs(5)),
                        create_sqlite_dirs: false,
                        ..Default::default()
                    };

                    tracing::info!("Connecting to mock database: sqlite::memory:");
                    let db = DbHandle::connect("sqlite::memory:", connect_opts).await?;
                    Ok(Arc::new(db))
                })
            });
            DbOptions::Auto(factory)
        } else {
            // Use new per-module database system
            tracing::info!("Using per-module database configuration");
            let factory = create_per_module_db_factory(Arc::new(config.clone()), base_dir.clone());
            DbOptions::PerModuleFactory(factory)
        }
    } else {
        tracing::warn!("No database configuration found, running without database");
        DbOptions::None
    };

    // Run the server via modkit
    let run_options: RunOptions = RunOptions {
        modules_cfg: config_provider,
        db: db_options,
        shutdown: ShutdownOptions::Signals,
    };

    run(run_options).await
}

async fn check_config(config: AppConfig) -> Result<()> {
    tracing::info!("Checking configuration...");

    // AppConfig::load_* already normalized & created home_dir
    tracing::info!("Configuration is valid");
    println!("Configuration check passed");
    println!("Server config:");
    println!("{}", config.to_yaml()?);

    Ok(())
}
