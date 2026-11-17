use std::time::Duration;

pub const READER_HZ: u32 = 125;
pub const MAX_DATA_AGE: Duration = Duration::from_millis(40);
pub const TRANSIENT_HOLD: Duration = Duration::from_millis(24);

pub fn retain_transient(status: &str, same_process: bool, has_frame: bool, age: Duration) -> bool {
    same_process
        && has_frame
        && age <= TRANSIENT_HOLD
        && (status.starts_with("CollectionChanged:") || status.starts_with("SingleAvatarChanged:"))
}

#[derive(Clone, Debug, Default)]
pub struct ReaderStats {
    pub polls: u64,
    pub accepted: u64,
    pub transient: u64,
    pub total_ms: f64,
    pub max_ms: f64,
}
impl ReaderStats {
    pub fn record(&mut self, duration: Duration, accepted: bool, transient: bool) {
        let ms = duration.as_secs_f64() * 1000.;
        self.polls = self.polls.saturating_add(1);
        self.accepted += u64::from(accepted);
        self.transient += u64::from(transient);
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }
    pub fn mean_ms(&self) -> f64 {
        if self.polls == 0 {
            0.
        } else {
            self.total_ms / self.polls as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brief_consistency_retry_keeps_only_the_same_live_process() {
        assert!(retain_transient(
            "CollectionChanged: retry",
            true,
            true,
            Duration::from_millis(8)
        ));
        assert!(!retain_transient(
            "CollectionChanged: retry",
            false,
            true,
            Duration::from_millis(8)
        ));
        assert!(!retain_transient(
            "CollectionChanged: retry",
            true,
            false,
            Duration::from_millis(8)
        ));
    }
    #[test]
    fn expired_or_identity_failure_frames_are_cleared() {
        assert!(!retain_transient(
            "CollectionChanged: retry",
            true,
            true,
            Duration::from_millis(25)
        ));
        for status in [
            "PointBlank check failed",
            "WaitingManager: invalid",
            "CollectionInvalid: pointer",
        ] {
            assert!(!retain_transient(
                status,
                true,
                true,
                Duration::from_millis(1)
            ));
        }
    }
    #[test]
    fn reader_metrics_report_actual_capture_cost() {
        let mut stats = ReaderStats::default();
        stats.record(Duration::from_micros(500), true, false);
        stats.record(Duration::from_micros(1500), false, true);
        assert_eq!((stats.polls, stats.accepted, stats.transient), (2, 1, 1));
        assert_eq!(stats.mean_ms(), 1.);
        assert_eq!(stats.max_ms, 1.5);
        assert_eq!(ReaderStats::default().mean_ms(), 0.);
    }
}
