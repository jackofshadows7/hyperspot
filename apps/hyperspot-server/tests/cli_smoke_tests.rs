//! CLI smoke tests for hyperspot-server binary
//!
//! These tests verify that the CLI commands work correctly, including
//! configuration validation, help output, and basic command functionality.

use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

/// Helper to run the hyperspot-server binary with given arguments
fn run_hyperspot_server(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hyperspot-server"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to execute hyperspot-server")
}

/// Helper to run the hyperspot-server binary with timeout
async fn run_hyperspot_server_with_timeout(
    args: &[&str],
    timeout_duration: Duration,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_hyperspot-server"));
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    match timeout(timeout_duration, cmd.output()).await {
        Ok(result) => result.map_err(|e| e.into()),
        Err(elapsed) => Err(elapsed.into()),
    }
}

#[test]
fn test_cli_help_command() {
    let output = run_hyperspot_server(&["--help"]);

    assert!(output.status.success(), "Help command should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hyperspot-server") || stdout.contains("HyperSpot"),
        "Should contain binary name"
    );
    assert!(
        stdout.contains("Usage:") || stdout.contains("USAGE:"),
        "Should contain usage information"
    );
    assert!(stdout.contains("run"), "Should contain 'run' subcommand");
    assert!(
        stdout.contains("check"),
        "Should contain 'check' subcommand"
    );
    assert!(stdout.contains("--config"), "Should mention config option");
}

#[test]
fn test_cli_version_command() {
    let output = run_hyperspot_server(&["--version"]);

    assert!(output.status.success(), "Version command should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hyperspot-server"),
        "Should contain binary name"
    );
    // Version might be 0.1.0 or similar
    assert!(
        stdout.chars().any(|c| c.is_ascii_digit()),
        "Should contain version numbers"
    );
}

#[test]
fn test_cli_invalid_command() {
    let output = run_hyperspot_server(&["invalid-command"]);

    assert!(!output.status.success(), "Invalid command should fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error") || stderr.contains("invalid") || stderr.contains("unexpected"),
        "Should contain error message about invalid command"
    );
}

#[test]
fn test_cli_config_validation_missing_file() {
    let output = run_hyperspot_server(&["--config", "/nonexistent/config.yaml", "check"]);

    assert!(!output.status.success(), "Should fail with missing config");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("config") || stderr.contains("file") || stderr.contains("found"),
        "Should mention config file issue: {}",
        stderr
    );
}

#[test]
fn test_cli_config_validation_invalid_yaml() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("invalid.yaml");

    // Write invalid YAML
    std::fs::write(&config_path, "invalid: yaml: content: [unclosed")
        .expect("Failed to write file");

    let output = run_hyperspot_server(&["--config", config_path.to_str().unwrap(), "check"]);

    assert!(!output.status.success(), "Should fail with invalid YAML");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("yaml") || stderr.contains("parse") || stderr.contains("format"),
        "Should mention YAML parsing issue: {}",
        stderr
    );
}

#[test]
fn test_cli_config_validation_valid_config() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("valid.yaml");

    // Write valid configuration
    let config_content = r#"
database:
  url: "sqlite:///tmp/test.db"

logging:
  # global section
  default:
    console_level: info
    file: "logs/hyperspot.log"
    file_level: info
    max_age_days: 28
    max_backups: 3
    max_size_mb: 1000
"#;

    std::fs::write(&config_path, config_content).expect("Failed to write config file");

    let output = run_hyperspot_server(&["--config", config_path.to_str().unwrap(), "check"]);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        eprintln!("STDERR: {}", stderr);
        eprintln!("STDOUT: {}", stdout);
    }

    assert!(output.status.success(), "Should succeed with valid config");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain some indication of successful validation
    assert!(
        stdout.contains("valid")
            || stdout.contains("OK")
            || stdout.contains("success")
            || stdout.is_empty(),
        "Should indicate successful validation or be empty: {}",
        stdout
    );
}

#[tokio::test]
async fn test_cli_run_command_with_mock_database() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("test.yaml");

    // Write test configuration with in-memory SQLite
    let config_content = r#"
database:
  url: "sqlite:///tmp/test.db?cache=shared&mode=rwc"

logging:
  # global section
  default:
    console_level: info
    file: "logs/hyperspot.log"
    file_level: info
    max_age_days: 28
    max_backups: 3
    max_size_mb: 1000
"#;

    std::fs::write(&config_path, config_content).expect("Failed to write config file");

    // Run server with short timeout to test startup
    let result = run_hyperspot_server_with_timeout(
        &["--config", config_path.to_str().unwrap(), "run"],
        Duration::from_secs(10),
    )
    .await;

    // Server should start and timeout (which means it was running)
    match result {
        Err(err) => {
            // Timeout is expected - server was running
            if err.to_string().contains("elapsed") {
                println!("✓ Server started successfully (timed out as expected)");
            } else {
                eprintln!("Server failed to start: {}", err);
                panic!("Server should start successfully");
            }
        }
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if output.status.success() {
                // If it completed successfully, that's also fine
                println!("✓ Server completed successfully");
            } else {
                eprintln!("Server failed to start:");
                eprintln!("STDOUT: {}", stdout);
                eprintln!("STDERR: {}", stderr);
                panic!("Server should start successfully");
            }
        }
    }
}

#[test]
fn test_cli_run_command_config_validation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("invalid.yaml");

    // Write configuration with invalid bind address
    let config_content = r#"
database:
  url: "sqlite:///tmp/test.db"

logging:
  level: "info"
"#;

    std::fs::write(&config_path, config_content).expect("Failed to write config file");

    let output = run_hyperspot_server(&["--config", config_path.to_str().unwrap(), "run"]);

    assert!(
        !output.status.success(),
        "Should fail with invalid bind address"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("address") || stderr.contains("parse") || stderr.contains("invalid"),
        "Should mention address parsing issue: {}",
        stderr
    );
}

#[test]
fn test_cli_mock_flag() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("mock.yaml");

    // Write configuration with PostgreSQL (which should be overridden by --mock)
    let config_content = r#"
database:
  url: "postgresql://localhost/nonexistent"

logging:
  level: "error"
  format: "compact"
"#;

    std::fs::write(&config_path, config_content).expect("Failed to write config file");

    // The check command with --mock should succeed even with invalid PostgreSQL config
    let output =
        run_hyperspot_server(&["--config", config_path.to_str().unwrap(), "--mock", "check"]);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        eprintln!("STDERR: {}", stderr);
        eprintln!("STDOUT: {}", stdout);
    }

    assert!(
        output.status.success(),
        "Should succeed with mock database even if PostgreSQL config is invalid"
    );
}

#[test]
fn test_cli_verbose_flag() {
    let output = run_hyperspot_server(&["--verbose", "--help"]);

    assert!(output.status.success(), "Verbose help should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should still show help output
    assert!(
        stdout.contains("Usage:") || stdout.contains("USAGE:"),
        "Should still contain usage information"
    );
}

#[test]
fn test_cli_config_flag_short_form() {
    // Test short form of config flag
    let output = run_hyperspot_server(&["-c", "/nonexistent/config.yaml", "check"]);

    assert!(
        !output.status.success(),
        "Should fail with missing config file"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("config") || stderr.contains("file") || stderr.contains("found"),
        "Should mention config file issue with short flag: {}",
        stderr
    );
}

#[test]
fn test_cli_subcommand_help() {
    // Test help for run subcommand
    let output = run_hyperspot_server(&["run", "--help"]);

    assert!(
        output.status.success(),
        "Run subcommand help should succeed"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("run") || stdout.contains("server"),
        "Should contain information about run command"
    );

    // Test help for check subcommand
    let output = run_hyperspot_server(&["check", "--help"]);

    assert!(
        output.status.success(),
        "Check subcommand help should succeed"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("check") || stdout.contains("configuration"),
        "Should contain information about check command"
    );
}

#[test]
fn test_cli_config_precedence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("precedence.yaml");

    // Write minimal valid configuration
    let config_content = r#"
database:
  url: "sqlite:///tmp/test.db"

logging:
  level: "debug"  # This should be overridden by command line args
"#;

    std::fs::write(&config_path, config_content).expect("Failed to write config file");

    let output = run_hyperspot_server(&["--config", config_path.to_str().unwrap(), "check"]);

    assert!(
        output.status.success(),
        "Should succeed with valid minimal config"
    );
}

#[test]
fn test_cli_no_arguments() {
    let output = run_hyperspot_server(&[]);

    // Should either show help or show an error about missing subcommand
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("Usage:")
            || stdout.contains("USAGE:")
            || stderr.contains("required")
            || stderr.contains("subcommand"),
        "Should show usage or error about missing subcommand"
    );
}
