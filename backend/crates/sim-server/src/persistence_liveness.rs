use std::sync::RwLock;
use std::time::{Duration, SystemTime};

/// Consecutive failed persist cycles tolerated while a recent success still holds
/// before the status drops from Healthy to Degraded.
const PERSIST_FAILURE_TOLERANCE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobilityPersistenceHealthStatus {
    Starting,
    Healthy,
    Degraded,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobilityPersistenceHealth {
    pub status: MobilityPersistenceHealthStatus,
    pub world_id: Option<String>,
    pub mobility_tick: Option<u64>,
    pub last_attempt: Option<SystemTime>,
    pub last_success: Option<SystemTime>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub freshness: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobilityPersistenceAttempt {
    world_id: String,
    mobility_tick: u64,
}

#[derive(Debug)]
pub struct MobilityPersistenceLiveness {
    freshness_window: Duration,
    inner: RwLock<MobilityPersistenceInner>,
}

#[derive(Debug, Default)]
struct MobilityPersistenceInner {
    world_id: Option<String>,
    mobility_tick: Option<u64>,
    last_attempt: Option<SystemTime>,
    last_success: Option<SystemTime>,
    consecutive_failures: u32,
    last_error: Option<String>,
}

impl MobilityPersistenceLiveness {
    pub fn new(freshness_window: Duration) -> Self {
        Self {
            freshness_window,
            inner: RwLock::new(MobilityPersistenceInner::default()),
        }
    }

    pub fn begin_attempt(
        &self,
        world_id: impl Into<String>,
        mobility_tick: u64,
        now: SystemTime,
    ) -> MobilityPersistenceAttempt {
        let world_id = world_id.into();
        let mut inner = self
            .inner
            .write()
            .expect("persistence liveness lock poisoned");
        inner.world_id = Some(world_id.clone());
        inner.mobility_tick = Some(mobility_tick);
        inner.last_attempt = Some(now);
        MobilityPersistenceAttempt {
            world_id,
            mobility_tick,
        }
    }

    pub fn record_success(&self, attempt: MobilityPersistenceAttempt, now: SystemTime) {
        let mut inner = self
            .inner
            .write()
            .expect("persistence liveness lock poisoned");
        inner.world_id = Some(attempt.world_id);
        inner.mobility_tick = Some(attempt.mobility_tick);
        inner.last_success = Some(now);
        inner.consecutive_failures = 0;
        inner.last_error = None;
    }

    pub fn record_failure(
        &self,
        attempt: MobilityPersistenceAttempt,
        error: impl AsRef<str>,
        now: SystemTime,
    ) {
        let mut inner = self
            .inner
            .write()
            .expect("persistence liveness lock poisoned");
        inner.world_id = Some(attempt.world_id);
        inner.mobility_tick = Some(attempt.mobility_tick);
        inner.consecutive_failures = inner.consecutive_failures.saturating_add(1);
        inner.last_error = Some(redact_persistence_error(error.as_ref()));
        inner.last_attempt = Some(now);
    }

    pub fn snapshot(&self) -> MobilityPersistenceHealth {
        self.snapshot_at(SystemTime::now())
    }

    pub fn snapshot_at(&self, now: SystemTime) -> MobilityPersistenceHealth {
        let inner = self
            .inner
            .read()
            .expect("persistence liveness lock poisoned");
        let freshness = inner
            .last_success
            .and_then(|last_success| now.duration_since(last_success).ok());
        let fresh = freshness.is_some_and(|age| age <= self.freshness_window);
        let status = match (inner.last_attempt, inner.last_success) {
            (None, None) => MobilityPersistenceHealthStatus::Starting,
            (_, Some(_)) if fresh && inner.consecutive_failures <= PERSIST_FAILURE_TOLERANCE => {
                MobilityPersistenceHealthStatus::Healthy
            }
            (_, Some(_)) if fresh => MobilityPersistenceHealthStatus::Degraded, // recent success but currently failing > tolerance
            (_, Some(_)) => MobilityPersistenceHealthStatus::Stale, // last success older than the window
            (Some(_), None) => MobilityPersistenceHealthStatus::Stale, // attempted, never succeeded → real outage
        };

        MobilityPersistenceHealth {
            status,
            world_id: inner.world_id.clone(),
            mobility_tick: inner.mobility_tick,
            last_attempt: inner.last_attempt,
            last_success: inner.last_success,
            consecutive_failures: inner.consecutive_failures,
            last_error: inner.last_error.clone(),
            freshness,
        }
    }
}

fn redact_persistence_error(message: &str) -> String {
    let mut out = message.to_owned();
    if let Some(start) = out.find("postgres://")
        && let Some(at) = out[start..].find('@')
    {
        let absolute_at = start + at;
        out.replace_range(start + "postgres://".len()..absolute_at, "<redacted>");
    }
    while let Some(start) = out.find("sb_secret_") {
        let end = out[start..]
            .find(char::is_whitespace)
            .map(|offset| start + offset)
            .unwrap_or(out.len());
        out.replace_range(start..end, "<redacted>");
    }
    out.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
    }

    #[test]
    fn starts_before_first_attempt() {
        let tracker = MobilityPersistenceLiveness::new(Duration::from_secs(15));

        let status = tracker.snapshot_at(at(1_000));

        assert_eq!(status.status, MobilityPersistenceHealthStatus::Starting);
        assert!(status.last_attempt.is_none());
        assert!(status.last_success.is_none());
        assert_eq!(status.consecutive_failures, 0);
    }

    #[test]
    fn success_is_healthy_until_freshness_window_expires() {
        let tracker = MobilityPersistenceLiveness::new(Duration::from_secs(15));

        let attempt = tracker.begin_attempt("abutopia", 42, at(1_000));
        tracker.record_success(attempt, at(1_100));

        let fresh = tracker.snapshot_at(at(16_000));
        assert_eq!(fresh.status, MobilityPersistenceHealthStatus::Healthy);
        assert_eq!(fresh.world_id.as_deref(), Some("abutopia"));
        assert_eq!(fresh.mobility_tick, Some(42));
        assert_eq!(fresh.freshness, Some(Duration::from_millis(14_900)));

        let stale = tracker.snapshot_at(at(16_101));
        assert_eq!(stale.status, MobilityPersistenceHealthStatus::Stale);
    }

    #[test]
    fn failure_without_prior_success_is_stale_and_redacted() {
        let tracker = MobilityPersistenceLiveness::new(Duration::from_secs(15));

        let attempt = tracker.begin_attempt("abutopia", 7, at(1_000));
        tracker.record_failure(
            attempt,
            "postgres://user:secret@example.test/db failed with token secret-token-123",
            at(1_050),
        );

        let status = tracker.snapshot_at(at(1_100));

        assert_eq!(status.status, MobilityPersistenceHealthStatus::Stale);
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.world_id.as_deref(), Some("abutopia"));
        assert_eq!(status.mobility_tick, Some(7));
        assert_eq!(
            status.last_error.as_deref(),
            Some("postgres://<redacted>@example.test/db failed with token secret-token-123")
        );
    }

    #[test]
    fn transient_failures_after_success_stay_healthy_within_tolerance() {
        let t = MobilityPersistenceLiveness::new(Duration::from_secs(15));
        let a = t.begin_attempt("abutopia", 1, at(0));
        t.record_success(a, at(0));
        let a = t.begin_attempt("abutopia", 2, at(1_000));
        t.record_failure(a, "boom", at(1_000)); // 1 failure, fresh success
        assert_eq!(
            t.snapshot_at(at(2_000)).status,
            MobilityPersistenceHealthStatus::Healthy
        );
    }

    #[test]
    fn sustained_failures_after_success_are_degraded_not_stale() {
        let t = MobilityPersistenceLiveness::new(Duration::from_secs(15));
        let a = t.begin_attempt("abutopia", 1, at(0));
        t.record_success(a, at(0));
        for ms in [1_000u64, 2_000, 3_000] {
            let a = t.begin_attempt("abutopia", 2, at(ms));
            t.record_failure(a, "boom", at(ms));
        }
        assert_eq!(
            t.snapshot_at(at(4_000)).status,
            MobilityPersistenceHealthStatus::Degraded // >2 failures, still fresh
        );
    }

    #[test]
    fn stale_success_is_stale_even_with_no_failures() {
        let t = MobilityPersistenceLiveness::new(Duration::from_secs(15));
        let a = t.begin_attempt("abutopia", 1, at(0));
        t.record_success(a, at(0));
        assert_eq!(
            t.snapshot_at(at(16_001)).status,
            MobilityPersistenceHealthStatus::Stale
        );
    }

    #[test]
    fn failure_redacts_all_supabase_secret_tokens() {
        let tracker = MobilityPersistenceLiveness::new(Duration::from_secs(15));

        let attempt = tracker.begin_attempt("abutopia", 8, at(1_000));
        tracker.record_failure(
            attempt,
            "request failed with sb_secret_first and sb_secret_second",
            at(1_050),
        );

        let status = tracker.snapshot_at(at(1_100));

        assert_eq!(
            status.last_error.as_deref(),
            Some("request failed with <redacted> and <redacted>")
        );
    }
}
