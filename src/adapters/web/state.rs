use chrono::{DateTime, Utc};

use crate::pipeline::PipelineReport;

// Re-export ScanProgress from pipeline (canonical location) for backward compatibility.
pub use crate::pipeline::ScanProgress;

/// Current status of a scan operation.
#[derive(Debug, Clone, Default)]
pub enum ScanStatus {
    #[default]
    Idle,
    Running {
        started_at: DateTime<Utc>,
    },
    Complete {
        finished_at: DateTime<Utc>,
        report: PipelineReport,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_status_default_is_idle() {
        let status = ScanStatus::default();
        assert!(matches!(status, ScanStatus::Idle));
    }

    #[test]
    fn scan_progress_serializes_to_json() {
        let progress = ScanProgress {
            stage: "source".into(),
            percent: 42,
            message: "Fetching feeds".into(),
        };
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"stage\":\"source\""));
        assert!(json.contains("\"percent\":42"));
        assert!(json.contains("\"message\":\"Fetching feeds\""));

        let deserialized: ScanProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.stage, "source");
        assert_eq!(deserialized.percent, 42);
        assert_eq!(deserialized.message, "Fetching feeds");
    }
}
