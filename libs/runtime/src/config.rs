use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// Uses your module: crate::home_dirs::resolve_home_dir
use crate::paths::home_dir::resolve_home_dir;

/// Main application configuration with strongly-typed global sections
/// and a flexible per-module configuration bag.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Core server configuration.
    pub server: ServerConfig,
    /// Database configuration (optional).
    pub database: Option<DatabaseConfig>,
    /// Logging configuration (optional, uses defaults if None).
    pub logging: Option<LoggingConfig>,
    /// Directory containing per-module YAML files (optional).
    #[serde(default)]
    pub modules_dir: Option<String>,
    /// Per-module configuration bag: module_name → arbitrary JSON/YAML value.
    #[serde(default)]
    pub modules: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub home_dir: String, // will be normalized to absolute path
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub timeout_sec: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Database connection URL (e.g., "sqlite://./db.sqlite", "postgres://user:pass@host/db").
    pub url: String,
    /// Maximum number of connections in the pool (optional, defaults to 10).
    pub max_conns: Option<u32>,
    /// SQLite busy timeout in milliseconds (optional, defaults to 5000).
    pub busy_timeout_ms: Option<u32>,
}

/// Logging configuration - maps subsystem names to their logging settings.
/// Key "default" is the catch-all for logs that don't match explicit subsystems.
pub type LoggingConfig = HashMap<String, Section>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Section {
    pub console_level: String, // "info", "debug", "error", "off"
    pub file: String,          // "logs/api.log"
    #[serde(default)]
    pub file_level: String,
    pub max_age_days: Option<u32>, // Not implemented yet
    #[serde(default)]
    pub max_backups: Option<usize>, // How many files to keep
    #[serde(default)]
    pub max_size_mb: Option<u64>, // Max size of the file in MB
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            // Empty => use platform default resolved by resolve_home_dir():
            // Windows: %APPDATA%/.hyperspot
            // Unix/macOS: $HOME/.hyperspot
            home_dir: String::new(),
            host: "127.0.0.1".to_string(),
            port: 8087,
            timeout_sec: 0,
        }
    }
}

/// Create a default logging configuration.
pub fn default_logging_config() -> LoggingConfig {
    let mut logging = HashMap::new();
    logging.insert(
        "default".to_string(),
        Section {
            console_level: "info".to_string(),
            file: "logs/hyperspot.log".to_string(),
            file_level: "debug".to_string(),
            max_age_days: Some(7),
            max_backups: Some(3),
            max_size_mb: Some(100),
        },
    );
    logging
}

impl Default for AppConfig {
    fn default() -> Self {
        let server = ServerConfig::default();
        Self {
            server,
            database: Some(DatabaseConfig {
                url: "sqlite://database/database.db".to_string(),
                max_conns: Some(10),
                busy_timeout_ms: Some(5000),
            }),
            logging: Some(default_logging_config()),
            modules_dir: None,
            modules: HashMap::new(),
        }
    }
}

impl AppConfig {
    /// Load configuration with layered loading: defaults → YAML file → environment variables.
    /// Also normalizes `server.home_dir` into an absolute path and creates the directory.
    pub fn load_layered<P: AsRef<Path>>(config_path: P) -> Result<Self> {
        use figment::{
            providers::{Env, Format, Serialized, Yaml},
            Figment,
        };

        // For layered loading, start from a minimal base where optional sections are None,
        // so they remain None unless explicitly provided by YAML/ENV.
        let base = AppConfig {
            server: ServerConfig::default(),
            database: None,
            logging: None,
            modules_dir: None,
            modules: HashMap::new(),
        };

        let figment = Figment::new()
            .merge(Serialized::defaults(base))
            .merge(Yaml::file(config_path.as_ref()))
            // Example: APP__SERVER__PORT=8087 maps to server.port
            .merge(Env::prefixed("APP__").split("__"));

        let mut config: AppConfig = figment
            .extract()
            .with_context(|| "Failed to extract config from figment".to_string())?;

        // Normalize + create home_dir immediately.
        normalize_home_dir_inplace(&mut config.server)
            .context("Failed to resolve server.home_dir")?;

        // Merge module files if modules_dir is specified.
        if let Some(dir) = config.modules_dir.clone() {
            merge_module_files(&mut config.modules, dir)?;
        }

        Ok(config)
    }

    /// Load configuration from file or create with default values.
    /// Also normalizes `server.home_dir` into an absolute path and creates the directory.
    pub fn load_or_default<P: AsRef<Path>>(config_path: Option<P>) -> Result<Self> {
        match config_path {
            Some(path) => Self::load_layered(path),
            None => {
                let mut c = Self::default();
                normalize_home_dir_inplace(&mut c.server)
                    .context("Failed to resolve server.home_dir (defaults)")?;
                Ok(c)
            }
        }
    }

    /// Serialize configuration to YAML.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self).context("Failed to serialize config to YAML")
    }

    /// Apply overrides from command line arguments.
    pub fn apply_cli_overrides(&mut self, args: &CliArgs) {
        if let Some(port) = args.port {
            self.server.port = port;
        }

        // Set logging level based on verbose flags for "default" section.
        let logging = self.logging.get_or_insert_with(default_logging_config);
        if let Some(default_section) = logging.get_mut("default") {
            default_section.console_level = match args.verbose {
                0 => default_section.console_level.clone(), // keep
                1 => "debug".to_string(),
                _ => "trace".to_string(),
            };
        }
    }
}

/// Command line arguments structure.
#[derive(Debug, Clone)]
pub struct CliArgs {
    pub config: Option<String>,
    pub port: Option<u16>,
    pub print_config: bool,
    pub verbose: u8,
    pub mock: bool,
}

// TODO: should be pass from outside
const fn default_subdir() -> &'static str {
    ".hyperspot"
}

/// Normalize `server.home_dir` using `home_dirs::resolve_home_dir` and store the absolute path back.
fn normalize_home_dir_inplace(server: &mut ServerConfig) -> Result<()> {
    // Treat empty string as "not provided" => None.
    let opt = if server.home_dir.trim().is_empty() {
        None
    } else {
        Some(server.home_dir.clone())
    };

    let resolved: PathBuf = resolve_home_dir(opt, default_subdir(), /*create*/ true)
        .context("home_dir normalization failed")?;

    server.home_dir = resolved.to_string_lossy().to_string();
    Ok(())
}

fn merge_module_files(
    bag: &mut HashMap<String, serde_json::Value>,
    dir: impl AsRef<Path>,
) -> Result<()> {
    use std::fs;
    let dir = dir.as_ref();
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "yml" && ext != "yaml" {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let raw = fs::read_to_string(&path)?;
        let val: serde_yaml::Value = serde_yaml::from_str(&raw)?;
        let json = serde_json::to_value(val)?;
        bag.insert(name, json);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};
    use tempfile::tempdir;

    /// Helper: a normalized home_dir should be absolute and not start with '~'.
    fn is_normalized_path(p: &str) -> bool {
        let pb = PathBuf::from(p);
        pb.is_absolute() && !p.starts_with('~')
    }

    /// Helper: platform default subdirectory name.
    fn default_subdir() -> &'static str {
        ".hyperspot"
    }

    #[test]
    fn test_default_config_structure() {
        let config = AppConfig::default();

        // Server defaults
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8087);
        // raw (not yet normalized)
        assert_eq!(config.server.home_dir, "");
        assert_eq!(config.server.timeout_sec, 0);

        // Database defaults
        assert!(config.database.is_some());
        let db = config.database.as_ref().unwrap();
        assert_eq!(db.url, "sqlite://database/database.db");
        assert_eq!(db.max_conns, Some(10));
        assert_eq!(db.busy_timeout_ms, Some(5000));

        // Logging defaults
        assert!(config.logging.is_some());
        let logging = config.logging.as_ref().unwrap();
        assert!(logging.contains_key("default"));

        let default_section = &logging["default"];
        assert_eq!(default_section.console_level, "info");
        assert_eq!(default_section.file, "logs/hyperspot.log");

        // Modules bag is empty by default
        assert!(config.modules.is_empty());
    }

    #[test]
    fn test_load_layered_normalizes_home_dir() {
        let tmp = tempdir().unwrap();
        let cfg_path = tmp.path().join("cfg.yaml");

        // Provide a user path with "~" to ensure expansion and normalization.
        let yaml = r#"
server:
  home_dir: "~/.test_hyperspot"
  host: "0.0.0.0"
  port: 9090
  timeout_sec: 30

database:
  url: "postgres://user:pass@localhost/db"
  max_conns: 20
  busy_timeout_ms: 10000

logging:
  default:
    console_level: debug
    file: "logs/default.log"
"#;
        fs::write(&cfg_path, yaml).unwrap();

        let config = AppConfig::load_layered(&cfg_path).unwrap();

        // home_dir should be normalized immediately
        assert!(is_normalized_path(&config.server.home_dir));
        assert!(config.server.home_dir.ends_with(".test_hyperspot"));
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.server.timeout_sec, 30);

        // database parsed
        let db = config.database.as_ref().unwrap();
        assert_eq!(db.url, "postgres://user:pass@localhost/db");
        assert_eq!(db.max_conns, Some(20));
        assert_eq!(db.busy_timeout_ms, Some(10000));

        // logging parsed
        let logging = config.logging.as_ref().unwrap();
        let def = &logging["default"];
        assert_eq!(def.console_level, "debug");
        assert_eq!(def.file, "logs/default.log");
    }

    #[test]
    fn test_load_or_default_normalizes_home_dir_when_none() {
        // No external file => defaults, but home_dir must be normalized.
        // Ensure platform env is present for home resolution in CI.
        let tmp = tempdir().unwrap();
        #[cfg(target_os = "windows")]
        env::set_var("APPDATA", tmp.path());
        #[cfg(not(target_os = "windows"))]
        env::set_var("HOME", tmp.path());
        let config = AppConfig::load_or_default(None::<&str>).unwrap();
        assert!(is_normalized_path(&config.server.home_dir));
        assert!(config.server.home_dir.ends_with(default_subdir()));
        assert_eq!(config.server.port, 8087);
    }

    #[test]
    fn test_minimal_yaml_config() {
        let tmp = tempdir().unwrap();
        let cfg_path = tmp.path().join("cfg.yaml");

        let yaml = r#"
server:
  home_dir: "~/.minimal"
  host: "localhost"
  port: 8080
"#;
        fs::write(&cfg_path, yaml).unwrap();

        let config = AppConfig::load_layered(&cfg_path).unwrap();

        // Required fields are parsed; home_dir normalized
        assert!(is_normalized_path(&config.server.home_dir));
        assert!(config.server.home_dir.ends_with(".minimal"));
        assert_eq!(config.server.host, "localhost");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.timeout_sec, 0);

        // Optional sections default to None
        assert!(config.database.is_none());
        assert!(config.logging.is_none());
        assert!(config.modules.is_empty());
    }

    #[test]
    fn test_cli_overrides() {
        let mut config = AppConfig::default();

        let args = super::CliArgs {
            config: None,
            port: Some(3000),
            print_config: false,
            verbose: 2, // trace
            mock: false,
        };

        config.apply_cli_overrides(&args);

        // Port override
        assert_eq!(config.server.port, 3000);

        // Verbose override affects logging
        let logging = config.logging.as_ref().unwrap();
        let default_section = &logging["default"];
        assert_eq!(default_section.console_level, "trace");
    }

    #[test]
    fn test_cli_verbose_levels_matrix() {
        for (verbose_level, expected_log_level) in [
            (0, "info"), // unchanged from default
            (1, "debug"),
            (2, "trace"),
            (3, "trace"), // cap at trace
        ] {
            let mut config = AppConfig::default();
            let args = super::CliArgs {
                config: None,
                port: None,
                print_config: false,
                verbose: verbose_level,
                mock: false,
            };

            config.apply_cli_overrides(&args);

            let logging = config.logging.as_ref().unwrap();
            let default_section = &logging["default"];

            if verbose_level == 0 {
                assert_eq!(default_section.console_level, "info");
            } else {
                assert_eq!(default_section.console_level, expected_log_level);
            }
        }
    }

    #[test]
    fn test_layered_config_loading_with_modules_dir() {
        let tmp = tempdir().unwrap();
        let cfg_path = tmp.path().join("modules_dir.yaml");
        let modules_dir = tmp.path().join("modules");

        fs::create_dir_all(&modules_dir).unwrap();
        let module_cfg = modules_dir.join("test_module.yaml");
        fs::write(
            &module_cfg,
            r#"
setting1: "value1"
setting2: 42
"#,
        )
        .unwrap();

        // Convert Windows paths to forward slashes for YAML compatibility
        let modules_dir_str = modules_dir.to_string_lossy().replace('\\', "/");
        let yaml = format!(
            r#"
server:
  home_dir: "~/.modules_test"
  host: "127.0.0.1"
  port: 8087

modules_dir: "{}"

modules:
  existing_module:
    key: "value"
"#,
            modules_dir_str
        );

        fs::write(&cfg_path, yaml).unwrap();

        let config = AppConfig::load_layered(&cfg_path).unwrap();

        // Should have loaded the existing module from modules section
        assert!(config.modules.contains_key("existing_module"));

        // Should have also loaded the module from modules_dir
        assert!(config.modules.contains_key("test_module"));

        // Check the loaded module config
        let test_module = &config.modules["test_module"];
        assert_eq!(test_module["setting1"], "value1");
        assert_eq!(test_module["setting2"], 42);
    }

    #[test]
    fn test_to_yaml_roundtrip_basic() {
        let config = AppConfig::default();
        let yaml = config.to_yaml().unwrap();
        assert!(yaml.contains("server:"));
        assert!(yaml.contains("database:"));
        assert!(yaml.contains("logging:"));

        let roundtrip: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(roundtrip.server.port, config.server.port);
    }

    #[test]
    fn test_invalid_yaml_missing_required_field() {
        let invalid_yaml = r#"
server:
  home_dir: "~/.test"
  # Missing required host field
  port: 8087
"#;

        let result: Result<AppConfig, _> = serde_yaml::from_str(invalid_yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_and_init_logging_smoke() {
        // Just verifies structure is acceptable for logging init path.
        let tmp = tempdir().unwrap();
        let cfg_path = tmp.path().join("logging.yaml");
        let yaml = r#"
server:
  home_dir: "~/.logging_test"
  host: "127.0.0.1"
  port: 8088

logging:
  default:
    console_level: debug
    file: ""
    file_level: info
"#;
        fs::write(&cfg_path, yaml).unwrap();

        let config = AppConfig::load_layered(&cfg_path).unwrap();
        assert!(config.logging.is_some());
        let logging = config.logging.as_ref().unwrap();
        assert!(logging.contains_key("default"));

        let default_section = &logging["default"];
        assert_eq!(default_section.console_level, "debug");
        assert_eq!(default_section.file_level, "info");
        // not calling init to avoid side effects in tests
    }
}
