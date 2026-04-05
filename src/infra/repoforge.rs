use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::infra::error::KbError;

/// Error type for the repoforge subprocess helper.
#[derive(Debug, thiserror::Error)]
pub enum RepoforgeRunnerError {
    #[error("repoforge timed out")]
    Timeout,
    #[error("repoforge exited with code {code}")]
    ProcessError { code: i32 },
    #[error("repoforge I/O error: {0}")]
    IoError(String),
}

impl From<RepoforgeRunnerError> for KbError {
    fn from(e: RepoforgeRunnerError) -> Self {
        KbError::RepoforgeExport {
            repo: String::new(),
            reason: e.to_string(),
        }
    }
}

/// Shared subprocess helper that wraps the `repoforge` CLI.
pub struct RepoforgeRunner {
    pub path: PathBuf,
    pub timeout: Duration,
}

impl RepoforgeRunner {
    /// Create a new runner pointing at the given binary path.
    #[must_use]
    pub fn new(path: PathBuf, timeout: Duration) -> Self {
        Self { path, timeout }
    }

    /// Run `repoforge export -w <dir> --compress -q` and return stdout.
    ///
    /// Used by `KbPipeline` — produces compressed output for LLM consumption.
    pub async fn export(&self, dir: &Path) -> Result<String, RepoforgeRunnerError> {
        self.run_export(dir, &["--compress", "-q"]).await
    }

    /// Run `repoforge export -w <dir> --no-contents -q` and return stdout.
    ///
    /// Used by `RepoforgeAnalyzer` — produces tree + definitions only.
    pub async fn export_no_contents(&self, dir: &Path) -> Result<String, RepoforgeRunnerError> {
        self.run_export(dir, &["--no-contents", "-q"]).await
    }

    async fn run_export(
        &self,
        dir: &Path,
        extra_args: &[&str],
    ) -> Result<String, RepoforgeRunnerError> {
        let future = tokio::process::Command::new(&self.path)
            .args(["export", "-w"])
            .arg(dir)
            .args(extra_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        let output = tokio::time::timeout(self.timeout, future)
            .await
            .map_err(|_| RepoforgeRunnerError::Timeout)?
            .map_err(|e| RepoforgeRunnerError::IoError(e.to_string()))?;

        if !output.status.success() {
            return Err(RepoforgeRunnerError::ProcessError {
                code: output.status.code().unwrap_or(-1),
            });
        }

        String::from_utf8(output.stdout)
            .map_err(|e| RepoforgeRunnerError::IoError(format!("invalid UTF-8: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures");
        p.push(name);
        p
    }

    fn fake_repoforge() -> PathBuf {
        fixture_path("fake_repoforge.sh")
    }

    fn fake_repoforge_bad() -> PathBuf {
        fixture_path("fake_repoforge_bad.sh")
    }

    fn tmp_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("failed to create temp dir")
    }

    #[tokio::test]
    async fn export_returns_stdout_on_success() {
        let runner = RepoforgeRunner::new(fake_repoforge(), Duration::from_secs(10));
        let dir = tmp_dir();
        let result = runner.export(dir.path()).await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        let output = result.unwrap();
        assert!(!output.is_empty(), "output should not be empty");
    }

    #[tokio::test]
    async fn export_no_contents_returns_stdout_on_success() {
        let runner = RepoforgeRunner::new(fake_repoforge(), Duration::from_secs(10));
        let dir = tmp_dir();
        let result = runner.export_no_contents(dir.path()).await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[tokio::test]
    async fn export_fails_on_process_error() {
        let runner = RepoforgeRunner::new(fake_repoforge_bad(), Duration::from_secs(10));
        let dir = tmp_dir();
        let result = runner.export(dir.path()).await;
        assert!(
            matches!(result, Err(RepoforgeRunnerError::ProcessError { .. })),
            "expected ProcessError, got {result:?}"
        );
    }

    #[tokio::test]
    async fn export_fails_on_timeout() {
        // Use a near-zero timeout with a script that sleeps
        let runner = RepoforgeRunner::new(
            fixture_path("fake_repoforge_slow.sh"),
            Duration::from_millis(50),
        );
        let dir = tmp_dir();
        let result = runner.export(dir.path()).await;
        assert!(
            matches!(result, Err(RepoforgeRunnerError::Timeout)),
            "expected Timeout, got {result:?}"
        );
    }

    #[test]
    fn runner_error_converts_to_kb_error() {
        let err = RepoforgeRunnerError::Timeout;
        let kb_err: KbError = err.into();
        assert!(
            matches!(kb_err, KbError::RepoforgeExport { .. }),
            "expected KbError::RepoforgeExport"
        );
    }
}
