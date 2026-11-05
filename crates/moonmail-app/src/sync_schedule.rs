use std::time::{Duration, Instant};

/// Monotonic, single-flight scheduling while the app is open. No timer catches
/// up missed intervals after sleep, and no submission work is ever scheduled.
#[derive(Default)]
pub struct SyncSchedule {
    minutes: u16,
    next: Option<Instant>,
    failures: u8,
    paused: bool,
}

impl SyncSchedule {
    pub fn configure(&mut self, minutes: u16, now: Instant) {
        if self.minutes == minutes {
            return;
        }
        self.minutes = minutes;
        self.paused = false;
        self.failures = 0;
        self.defer(now);
    }
    pub fn due(&self, now: Instant) -> bool {
        !self.paused && self.next.is_some_and(|next| now >= next)
    }
    pub fn defer(&mut self, now: Instant) {
        self.next = (self.minutes > 0 && !self.paused)
            .then(|| now + Duration::from_secs(u64::from(self.minutes) * 60));
    }
    pub fn started(&mut self, manual: bool) {
        self.next = None;
        if manual {
            self.paused = false;
            self.failures = 0;
        }
    }
    pub fn pause(&mut self) {
        self.paused = true;
        self.next = None;
    }
    pub fn finished(&mut self, success: bool, retryable: bool, now: Instant) {
        if self.paused || self.minutes == 0 {
            return;
        }
        if success {
            self.failures = 0;
            self.defer(now);
        } else if retryable {
            self.failures = self.failures.saturating_add(1);
            let seconds = (30u64 << self.failures.saturating_sub(1).min(5)).min(900);
            self.next = Some(now + Duration::from_secs(seconds));
        } else {
            self.pause();
        }
    }
    pub fn label(&self) -> String {
        if self.minutes == 0 {
            "Manual sync".into()
        } else if self.paused {
            "Automatic sync paused · use Sync to resume".into()
        } else if self.failures > 0 {
            format!("Reconnect backoff · {} consecutive failures", self.failures)
        } else {
            format!("Automatic sync every {} min while open", self.minutes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheduling_is_single_flight_and_sleep_never_replays_missed_intervals() {
        let now = Instant::now();
        let mut schedule = SyncSchedule::default();
        assert!(!schedule.due(now + Duration::from_secs(10000)));
        schedule.configure(5, now);
        assert!(!schedule.due(now + Duration::from_secs(299)));
        let wake = now + Duration::from_secs(10000);
        assert!(schedule.due(wake));
        schedule.started(false);
        assert!(!schedule.due(wake));
        schedule.finished(true, false, wake);
        assert!(!schedule.due(wake + Duration::from_secs(299)));
        assert!(schedule.due(wake + Duration::from_secs(300)));
    }
    #[test]
    fn only_transient_errors_retry_with_capped_backoff_and_manual_sync_resumes_pause() {
        let mut now = Instant::now();
        let mut schedule = SyncSchedule::default();
        schedule.configure(5, now);
        for seconds in [30, 60, 120, 240, 480, 900, 900] {
            schedule.started(false);
            schedule.finished(false, true, now);
            assert!(!schedule.due(now + Duration::from_secs(seconds - 1)));
            now += Duration::from_secs(seconds);
            assert!(schedule.due(now));
        }
        schedule.started(false);
        schedule.finished(false, false, now);
        assert!(!schedule.due(now + Duration::from_secs(10000)));
        schedule.started(true);
        schedule.finished(false, true, now);
        assert!(schedule.due(now + Duration::from_secs(30)));
        schedule.pause();
        schedule.finished(true, false, now);
        assert!(!schedule.due(now + Duration::from_secs(10000)));
        schedule.started(true);
        schedule.finished(true, false, now);
        assert!(schedule.due(now + Duration::from_secs(300)));
        schedule.configure(0, now);
        schedule.finished(false, true, now);
        assert!(!schedule.due(now + Duration::from_secs(10000)));
    }
}
