use assert_cmd::Command;
use clap::Parser;
use predicates::prelude::*;
use repo_radar::cli::Cli;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Clap parsing tests (no binary execution, no network)
// ---------------------------------------------------------------------------

#[test]
fn parse_scan_defaults() {
    let cli = Cli::try_parse_from(["repo-radar", "scan"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Scan {
            dry_run,
            stage,
            backfill,
            accumulate,
            kb_path,
        } => {
            assert!(!dry_run);
            assert!(stage.is_none());
            assert!(!backfill);
            assert!(!accumulate);
            assert!(kb_path.is_none());
        }
        other => panic!("expected Scan, got {other:?}"),
    }
}

#[test]
fn parse_scan_dry_run() {
    let cli = Cli::try_parse_from(["repo-radar", "scan", "--dry-run"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Scan { dry_run, .. } => assert!(dry_run),
        other => panic!("expected Scan, got {other:?}"),
    }
}

#[test]
fn parse_scan_stage_filter() {
    let cli = Cli::try_parse_from(["repo-radar", "scan", "--stage", "source"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Scan { stage, .. } => {
            assert_eq!(stage.as_deref(), Some("source"));
        }
        other => panic!("expected Scan, got {other:?}"),
    }
}

#[test]
fn parse_scan_backfill() {
    let cli = Cli::try_parse_from(["repo-radar", "scan", "--backfill"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Scan { backfill, .. } => assert!(backfill),
        other => panic!("expected Scan, got {other:?}"),
    }
}

#[test]
fn parse_config_init() {
    let cli = Cli::try_parse_from(["repo-radar", "config", "init"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Config {
            action: repo_radar::cli::ConfigAction::Init,
        } => {}
        other => panic!("expected Config Init, got {other:?}"),
    }
}

#[test]
fn parse_config_show() {
    let cli = Cli::try_parse_from(["repo-radar", "config", "show"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Config {
            action: repo_radar::cli::ConfigAction::Show,
        } => {}
        other => panic!("expected Config Show, got {other:?}"),
    }
}

#[test]
fn parse_report_defaults() {
    let cli = Cli::try_parse_from(["repo-radar", "report"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Report { format, output } => {
            assert_eq!(format, "markdown");
            assert!(output.is_none());
        }
        other => panic!("expected Report, got {other:?}"),
    }
}

#[test]
fn parse_report_json_format() {
    let cli = Cli::try_parse_from(["repo-radar", "report", "--format", "json"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Report { format, .. } => {
            assert_eq!(format, "json");
        }
        other => panic!("expected Report, got {other:?}"),
    }
}

#[test]
fn parse_report_output_dir() {
    let cli =
        Cli::try_parse_from(["repo-radar", "report", "--output", "/tmp/reports"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Report { output, .. } => {
            assert_eq!(
                output.as_deref(),
                Some(std::path::Path::new("/tmp/reports"))
            );
        }
        other => panic!("expected Report, got {other:?}"),
    }
}

#[test]
fn parse_global_config_flag() {
    let cli =
        Cli::try_parse_from(["repo-radar", "--config", "/tmp/my.toml", "scan"]).unwrap();
    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new("/tmp/my.toml"))
    );
}

#[test]
fn parse_verbose_flags() {
    let cli = Cli::try_parse_from(["repo-radar", "-vv", "scan"]).unwrap();
    assert_eq!(cli.verbose, 2);
}

#[test]
fn parse_serve_defaults() {
    let cli = Cli::try_parse_from(["repo-radar", "serve"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Serve { port, host } => {
            assert_eq!(port, 3000);
            assert_eq!(host, "127.0.0.1");
        }
        other => panic!("expected Serve, got {other:?}"),
    }
}

#[test]
fn parse_serve_custom_port() {
    let cli = Cli::try_parse_from(["repo-radar", "serve", "--port", "8080"]).unwrap();
    match cli.command {
        repo_radar::cli::Command::Serve { port, host } => {
            assert_eq!(port, 8080);
            assert_eq!(host, "127.0.0.1");
        }
        other => panic!("expected Serve, got {other:?}"),
    }
}

#[test]
fn parse_serve_custom_host_and_port() {
    let cli = Cli::try_parse_from([
        "repo-radar",
        "serve",
        "--port",
        "8080",
        "--host",
        "0.0.0.0",
    ])
    .unwrap();
    match cli.command {
        repo_radar::cli::Command::Serve { port, host } => {
            assert_eq!(port, 8080);
            assert_eq!(host, "0.0.0.0");
        }
        other => panic!("expected Serve, got {other:?}"),
    }
}

#[test]
fn parse_unknown_subcommand_fails() {
    let result = Cli::try_parse_from(["repo-radar", "foobar"]);
    assert!(result.is_err());
}

#[test]
fn parse_no_subcommand_fails() {
    let result = Cli::try_parse_from(["repo-radar"]);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Binary integration tests (assert_cmd)
// ---------------------------------------------------------------------------

#[test]
fn cli_help_flag_exits_successfully() {
    Command::cargo_bin("repo-radar")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-radar"))
        .stdout(predicate::str::contains("Feed-driven GitHub repo discovery engine"));
}

#[test]
fn cli_version_flag_exits_successfully() {
    Command::cargo_bin("repo-radar")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-radar"));
}

#[test]
fn cli_unknown_subcommand_fails() {
    Command::cargo_bin("repo-radar")
        .unwrap()
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

/// `config init` without --config writes to the XDG default path.
/// We only verify the command exits 0
/// (it prints "already exists" or "created" depending on disk state).
#[test]
fn cli_config_init_exits_successfully() {
    Command::cargo_bin("repo-radar")
        .unwrap()
        .args(["config", "init"])
        .assert()
        .success();
}

/// `config init --config <path>` creates the config at the specified path.
#[test]
fn cli_config_init_respects_config_flag() {
    let tmp = TempDir::new().unwrap();
    let config_file = tmp.path().join("custom.toml");

    assert!(!config_file.exists());

    Command::cargo_bin("repo-radar")
        .unwrap()
        .args([
            "--config",
            config_file.to_str().unwrap(),
            "config",
            "init",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created default config"));

    assert!(config_file.exists());
}

/// `config show` with --config loads from the specified path.
#[test]
fn cli_config_show_outputs_toml() {
    let tmp = TempDir::new().unwrap();
    let config_file = tmp.path().join("show.toml");

    // First, create a config at the custom path.
    Command::cargo_bin("repo-radar")
        .unwrap()
        .args([
            "--config",
            config_file.to_str().unwrap(),
            "config",
            "init",
        ])
        .assert()
        .success();

    // Now show it using --config.
    Command::cargo_bin("repo-radar")
        .unwrap()
        .args([
            "--config",
            config_file.to_str().unwrap(),
            "config",
            "show",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[reporter]"));
}

/// `scan --dry-run` with a valid config file shows the resolved TOML.
#[test]
fn cli_scan_dry_run_with_config() {
    let tmp = TempDir::new().unwrap();
    let config_file = tmp.path().join("config.toml");

    // Write a minimal valid config so scan can load it.
    std::fs::write(&config_file, "[general]\n").unwrap();

    Command::cargo_bin("repo-radar")
        .unwrap()
        .args([
            "--config",
            config_file.to_str().unwrap(),
            "scan",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));
}

#[test]
fn cli_scan_without_config_shows_welcome() {
    let tmp = TempDir::new().unwrap();
    let config_file = tmp.path().join("nonexistent.toml");

    Command::cargo_bin("repo-radar")
        .unwrap()
        .args(["--config", config_file.to_str().unwrap(), "scan"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Welcome to repo-radar"));
}
