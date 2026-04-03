use chrono::{DateTime, Utc};
use tracing::warn;

/// Tracks GitHub API rate limit state to warn when approaching the limit.
#[derive(Debug)]
pub struct RateLimitTracker {
    remaining: Option<u32>,
    reset_at: Option<DateTime<Utc>>,
    threshold: u32,
}

impl RateLimitTracker {
    /// Create a new tracker with the given warning threshold.
    #[must_use]
    pub fn new(threshold: u32) -> Self {
        Self {
            remaining: None,
            reset_at: None,
            threshold,
        }
    }

    /// Update rate limit state from API response headers.
    pub fn update(&mut self, remaining: u32, reset_at: DateTime<Utc>) {
        self.remaining = Some(remaining);
        self.reset_at = Some(reset_at);

        if remaining < self.threshold {
            warn!(
                remaining,
                threshold = self.threshold,
                reset_at = %reset_at,
                "GitHub API rate limit is low"
            );
        }
    }

    /// Whether the remaining calls are below the configured threshold.
    #[must_use]
    pub fn is_low(&self) -> bool {
        self.remaining.is_some_and(|r| r < self.threshold)
    }

    /// Current remaining API calls, if known.
    #[must_use]
    pub fn remaining(&self) -> Option<u32> {
        self.remaining
    }

    /// When the rate limit resets, if known.
    #[must_use]
    pub fn reset_at(&self) -> Option<DateTime<Utc>> {
        self.reset_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_has_no_state() {
        let tracker = RateLimitTracker::new(100);
        assert!(tracker.remaining().is_none());
        assert!(tracker.reset_at().is_none());
        assert!(!tracker.is_low());
    }

    #[test]
    fn update_sets_state() {
        let mut tracker = RateLimitTracker::new(100);
        let reset = Utc::now();
        tracker.update(4500, reset);

        assert_eq!(tracker.remaining(), Some(4500));
        assert_eq!(tracker.reset_at(), Some(reset));
        assert!(!tracker.is_low());
    }

    #[test]
    fn is_low_below_threshold() {
        let mut tracker = RateLimitTracker::new(100);
        tracker.update(50, Utc::now());
        assert!(tracker.is_low());
    }

    #[test]
    fn is_low_at_threshold_is_false() {
        let mut tracker = RateLimitTracker::new(100);
        tracker.update(100, Utc::now());
        assert!(!tracker.is_low());
    }

    #[test]
    fn is_low_above_threshold() {
        let mut tracker = RateLimitTracker::new(100);
        tracker.update(101, Utc::now());
        assert!(!tracker.is_low());
    }

    #[test]
    fn update_overwrites_previous_state() {
        let mut tracker = RateLimitTracker::new(100);
        tracker.update(4000, Utc::now());
        assert_eq!(tracker.remaining(), Some(4000));

        tracker.update(3999, Utc::now());
        assert_eq!(tracker.remaining(), Some(3999));
    }

    #[test]
    fn zero_remaining_is_low() {
        let mut tracker = RateLimitTracker::new(100);
        tracker.update(0, Utc::now());
        assert!(tracker.is_low());
    }

    #[test]
    fn threshold_zero_never_low() {
        let mut tracker = RateLimitTracker::new(0);
        tracker.update(0, Utc::now());
        assert!(!tracker.is_low());
    }
}
