//! Notices when the proxy cannot carry traffic, so it can stop holding the
//! machine hostage.
//!
//! Lumen puts itself in the path of every request by setting the system proxy,
//! and a watchdog re-asserts that setting every twenty seconds. Nothing in that
//! loop ever asked whether the daemon could actually serve a request. When the
//! upstream leg broke — a second interceptor mangling HTTP/2, a captive
//! network, a dead route — every client on the machine broke with it, and
//! restarting Lumen only re-established the same broken state. Clearing the
//! proxy by hand did not survive the next watchdog tick.
//!
//! The effect is a machine-wide single point of failure that fights recovery.
//! This watcher is the missing half: count what the proxy is actually doing,
//! and let the UI say "capture is failing" and offer the way out, instead of
//! leaving someone to guess which of Lumen, the network and the client is at
//! fault while nothing loads.
//!
//! Deliberately a *soft* signal, like `auth_watch`. A handful of upstream
//! failures is normal — a flaky endpoint, a request cancelled mid-flight, a
//! host that is genuinely down. Only a sustained run with no success in
//! between counts, and any success clears it, because the question is "is
//! anything getting through" rather than "did something fail".
//!
//! Cost per request: one atomic store on success, two on failure.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Consecutive failures, with no success in between, before the UI is told.
///
/// Three rather than one: a single failure is ordinary, and the cost of a false
/// amber is a user pulling a working proxy out of their own path.
const FAILURE_STREAK: u64 = 3;

/// A streak older than this stops counting. A laptop closed mid-failure and
/// opened somewhere else should not wake up amber on evidence from yesterday.
const STREAK_DECAY_SECS: u64 = 10 * 60;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct UpstreamHealth {
    consecutive_failures: AtomicU64,
    last_failure_at: AtomicU64,
    last_success_at: AtomicU64,
    /// The most recent failure, already flattened through the error chain, so
    /// the UI can say what went wrong rather than only that something did.
    last_error: RwLock<Option<String>>,
}

impl UpstreamHealth {
    pub fn new() -> Self {
        Self::default()
    }

    /// One upstream request completed. Any response at all counts, including a
    /// 500: the question is whether bytes move through the proxy, not whether
    /// the origin is happy.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.last_success_at.store(now_secs(), Ordering::Relaxed);
    }

    /// One upstream request failed before producing a response.
    pub fn record_failure(&self, detail: &str) {
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        self.last_failure_at.store(now_secs(), Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.write() {
            *slot = Some(detail.to_string());
        }
    }

    /// True when a sustained run of failures is still recent.
    pub fn degraded(&self) -> bool {
        if self.consecutive_failures.load(Ordering::Relaxed) < FAILURE_STREAK {
            return false;
        }
        let last = self.last_failure_at.load(Ordering::Relaxed);
        last != 0 && now_secs().saturating_sub(last) <= STREAK_DECAY_SECS
    }

    pub fn consecutive_failures(&self) -> u64 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().ok().and_then(|s| s.clone())
    }

    /// Seconds since a request last got through, or `None` if none ever has.
    /// `None` on a daemon that has been up a while is itself the tell.
    pub fn last_success_ago_secs(&self) -> Option<u64> {
        match self.last_success_at.load(Ordering::Relaxed) {
            0 => None,
            at => Some(now_secs().saturating_sub(at)),
        }
    }

    /// Forget the streak. For the UI's "try again" — the user has changed
    /// something and wants a fresh verdict, not the old one decaying.
    pub fn reset(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.write() {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_failure_is_not_a_verdict() {
        let h = UpstreamHealth::new();
        h.record_failure("dns error");
        assert!(!h.degraded(), "one failure is ordinary");
    }

    #[test]
    fn a_sustained_run_with_no_success_flags() {
        let h = UpstreamHealth::new();
        for _ in 0..FAILURE_STREAK {
            h.record_failure("http2 protocol error");
        }
        assert!(h.degraded());
        assert_eq!(h.last_error().as_deref(), Some("http2 protocol error"));
    }

    #[test]
    fn one_success_clears_it() {
        // The question is whether anything is getting through, so a single
        // request that does is enough to stop accusing the proxy.
        let h = UpstreamHealth::new();
        for _ in 0..FAILURE_STREAK * 2 {
            h.record_failure("refused");
        }
        assert!(h.degraded());

        h.record_success();
        assert!(!h.degraded());
        assert_eq!(h.consecutive_failures(), 0);
    }

    #[test]
    fn never_having_succeeded_is_distinguishable_from_succeeding_recently() {
        let h = UpstreamHealth::new();
        assert_eq!(h.last_success_ago_secs(), None);
        h.record_success();
        assert!(h.last_success_ago_secs().is_some());
    }

    #[test]
    fn reset_drops_the_streak_and_the_error() {
        let h = UpstreamHealth::new();
        for _ in 0..FAILURE_STREAK {
            h.record_failure("refused");
        }
        h.reset();
        assert!(!h.degraded());
        assert!(h.last_error().is_none());
    }
}
