//! When was anything last captured?
//!
//! Proxy mode costs the user a trusted root in their login keychain, and that
//! trust outlives the app: uninstalling Lumen does not take it back out. The CA
//! is minted with a ten-year validity, so expiry — which used to clean up after
//! a lapsed user eventually — now never will.
//!
//! The user who presses "Remove Certificate" was never the problem. The one who
//! stops using Lumen and forgets about it is, and the only signal that says so
//! is that nothing has been captured for a long time. This module is that
//! signal: one Unix timestamp, updated on every captured call, persisted
//! coarsely so it survives daemon restarts, and reported on `/ca/info` for the
//! app to turn into an advisory next to the button that already does the work.
//!
//! It advises and nothing more. Nothing here deletes a certificate, drops trust,
//! or changes capture behaviour.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// How long the daemon must go without capturing anything before the app
/// suggests taking the certificate back out of the keychain.
///
/// Ninety days is well past a holiday, a long stretch on another machine, or a
/// season spent on work that never touches an LLM — so crossing it means
/// "probably done with proxy mode", not "away for a bit".
///
/// This crate owns the number. It is reported on `/ca/info` as
/// `idle_advisory_days` so the app compares against the daemon's threshold
/// rather than keeping a second copy that can drift from this one.
pub const IDLE_ADVISORY_DAYS: i64 = 90;

/// Coarseness of the on-disk copy.
///
/// `record_capture` runs on every intercepted call, which on a busy Claude Code
/// session is several times a second — writing through each time would turn a
/// timestamp into a filesystem write per API request. The in-memory value stays
/// exact; only the persisted copy lags, and it is read once at startup to answer
/// a question measured in months.
const PERSIST_INTERVAL_SECS: i64 = 15 * 60;

const SECS_PER_DAY: i64 = 86_400;

/// On-disk shape of `~/.lumen/activity.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ActivityFile {
    /// Unix seconds of the last captured call. `None` in a file that predates
    /// the field, which reads identically to "nothing has been captured".
    #[serde(default)]
    last_capture_at: Option<i64>,
}

/// The last-capture timestamp, in memory and (coarsely) on disk.
pub struct CaptureActivity {
    /// Unix seconds of the most recent capture. `0` means none recorded —
    /// a sentinel rather than an `Option` so the proxy hot path is a single
    /// relaxed atomic store instead of taking a lock.
    last_capture_at: AtomicI64,
    /// Unix seconds of the last write to `path`, or `0` if this process has not
    /// written yet. Drives the debounce.
    last_persisted_at: AtomicI64,
    /// `None` when there is nowhere to write — no home directory, or a file we
    /// could not create. The value is still tracked for this run; it just does
    /// not survive a restart, which degrades the advisory rather than the
    /// daemon.
    path: Option<PathBuf>,
}

impl CaptureActivity {
    /// Disk-backed at `~/.lumen/activity.json`, seeded with whatever the
    /// previous run left there.
    ///
    /// A missing, unreadable, or corrupt file is not an error: it resolves to
    /// "nothing captured yet", the same as a first run. Refusing to boot over an
    /// advisory's bookkeeping would be a far worse failure than losing the
    /// advisory.
    pub fn load() -> Self {
        Self::open(crate::state::activity_path())
    }

    fn open(path: Option<PathBuf>) -> Self {
        let last = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<ActivityFile>(&bytes).ok())
            .and_then(|file| file.last_capture_at)
            // A negative or absurdly future timestamp is a corrupt file by
            // another name; treat it as no record rather than reporting
            // nonsense idleness.
            .filter(|ts| *ts > 0)
            .unwrap_or(0);

        Self {
            last_capture_at: AtomicI64::new(last),
            last_persisted_at: AtomicI64::new(0),
            path,
        }
    }

    /// Note that something was just captured.
    pub fn record(&self) {
        self.record_at(now_unix());
    }

    /// `record` with the clock supplied, so the debounce is testable without
    /// waiting fifteen minutes.
    pub fn record_at(&self, now: i64) {
        self.last_capture_at.store(now, Ordering::Relaxed);

        let last_write = self.last_persisted_at.load(Ordering::Relaxed);
        if last_write != 0 && now - last_write < PERSIST_INTERVAL_SECS {
            return;
        }
        // Claim the write before doing it, so concurrent captures produce one
        // write rather than one each.
        if self
            .last_persisted_at
            .compare_exchange(last_write, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        self.persist(now);
    }

    /// Unix seconds of the last captured call, or `None` if nothing ever was.
    pub fn last_capture_at(&self) -> Option<i64> {
        match self.last_capture_at.load(Ordering::Relaxed) {
            0 => None,
            ts => Some(ts),
        }
    }

    fn persist(&self, ts: i64) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = ActivityFile {
            last_capture_at: Some(ts),
        };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// The process-wide tracker, installed by `main` at startup.
///
/// Global for the same reason the pricing database is loaded once: the value is
/// a property of this daemon, and threading it through the proxy, the
/// transparent proxy, and the passive sniffer would add a parameter to three
/// call chains to say the same thing. Left uninstalled — as it is in unit tests
/// — recording is a no-op, so tests never touch the user's home directory.
static ACTIVITY: OnceLock<CaptureActivity> = OnceLock::new();

/// Install the disk-backed tracker. Call once at startup, before the proxy runs.
pub fn install() {
    let _ = ACTIVITY.set(CaptureActivity::load());
}

/// Note that something was just captured. No-op until `install` has run.
pub fn record_capture() {
    if let Some(activity) = ACTIVITY.get() {
        activity.record();
    }
}

/// Unix seconds of the last captured call, or `None` if nothing ever was.
pub fn last_capture_at() -> Option<i64> {
    ACTIVITY.get().and_then(|a| a.last_capture_at())
}

/// Whole days since anything was captured, for `/ca/info`.
///
/// Falls back to the CA's own `notBefore` when nothing has ever been captured.
/// That fallback is the point of the feature, not a nicety: a root trusted six
/// months ago and never used once is the clearest case of a certificate the user
/// has no use for, and reporting `None` there would hide exactly the users worth
/// telling. `None` is returned only when there is no CA and no capture at all —
/// nothing was installed, so nothing is owed.
pub fn idle_days(
    last_capture_at: Option<i64>,
    ca_not_before: Option<i64>,
    now: i64,
) -> Option<i64> {
    let since = last_capture_at.or(ca_not_before)?;
    // Clamped at zero: a clock that moved backwards, or the one-hour backdate on
    // a freshly minted CA, must not read as negative idleness.
    Some((now - since).max(0) / SECS_PER_DAY)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plausible wall-clock instant to anchor the tests on. Real magnitudes
    /// matter: the debounce arithmetic is done in absolute seconds.
    const NOW: i64 = 1_780_000_000;

    fn at_path(path: PathBuf) -> CaptureActivity {
        CaptureActivity::open(Some(path))
    }

    fn read_persisted(path: &PathBuf) -> Option<i64> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice::<ActivityFile>(&bytes)
            .ok()?
            .last_capture_at
    }

    #[test]
    fn idle_days_from_a_recent_capture_is_below_the_threshold() {
        let three_days_ago = NOW - 3 * SECS_PER_DAY;
        let days = idle_days(Some(three_days_ago), Some(NOW - 400 * SECS_PER_DAY), NOW);

        assert_eq!(days, Some(3));
        assert!(
            days.unwrap() < IDLE_ADVISORY_DAYS,
            "must not advise removal"
        );
    }

    #[test]
    fn idle_days_from_an_old_capture_is_above_the_threshold() {
        let days = idle_days(
            Some(NOW - 200 * SECS_PER_DAY),
            Some(NOW - 400 * SECS_PER_DAY),
            NOW,
        );

        assert_eq!(days, Some(200));
        assert!(days.unwrap() >= IDLE_ADVISORY_DAYS, "must advise removal");
    }

    /// The case the feature exists for: trusted long ago, never once used.
    /// Falling back to `None` here would leave that user unwarned forever.
    #[test]
    fn idle_days_falls_back_to_the_ca_creation_date_when_nothing_was_captured() {
        let days = idle_days(None, Some(NOW - 180 * SECS_PER_DAY), NOW);

        assert_eq!(days, Some(180));
        assert!(days.unwrap() >= IDLE_ADVISORY_DAYS);
    }

    #[test]
    fn idle_days_is_none_only_when_there_is_no_ca_and_no_capture() {
        assert_eq!(idle_days(None, None, NOW), None);
        // A capture with an unreadable CA still yields an answer.
        assert_eq!(idle_days(Some(NOW - SECS_PER_DAY), None, NOW), Some(1));
    }

    #[test]
    fn idle_days_never_goes_negative() {
        // A CA backdated for clock skew, and a clock that jumped backwards.
        assert_eq!(idle_days(None, Some(NOW + 3600), NOW), Some(0));
        assert_eq!(idle_days(Some(NOW + 10 * SECS_PER_DAY), None, NOW), Some(0));
    }

    #[test]
    fn a_capture_is_recorded_and_persisted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".lumen").join("activity.json");

        let activity = at_path(path.clone());
        assert_eq!(activity.last_capture_at(), None, "nothing captured yet");

        activity.record_at(NOW);

        assert_eq!(activity.last_capture_at(), Some(NOW));
        assert_eq!(
            read_persisted(&path),
            Some(NOW),
            "the first capture writes through"
        );
    }

    /// Idleness must be measured from the last real capture, not from the last
    /// daemon start — otherwise restarting resets it and the advisory, which
    /// needs ninety uninterrupted days, never fires at all.
    #[test]
    fn the_last_capture_survives_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("activity.json");

        at_path(path.clone()).record_at(NOW);

        let after_restart = at_path(path);
        assert_eq!(after_restart.last_capture_at(), Some(NOW));
    }

    /// The debounce: `record_at` runs on every intercepted call, so the disk
    /// copy is written at most once per window while the in-memory value keeps
    /// tracking every single capture exactly.
    #[test]
    fn the_write_is_debounced_but_the_memory_value_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("activity.json");
        let activity = at_path(path.clone());

        activity.record_at(NOW);
        assert_eq!(read_persisted(&path), Some(NOW));

        // Several captures inside the window: memory follows each one, disk does
        // not move off the first write.
        for offset in [1, 60, 300, PERSIST_INTERVAL_SECS - 1] {
            activity.record_at(NOW + offset);
            assert_eq!(activity.last_capture_at(), Some(NOW + offset));
            assert_eq!(
                read_persisted(&path),
                Some(NOW),
                "no second write inside the {PERSIST_INTERVAL_SECS}s window"
            );
        }

        // Past the window, the disk copy catches up.
        activity.record_at(NOW + PERSIST_INTERVAL_SECS);
        assert_eq!(read_persisted(&path), Some(NOW + PERSIST_INTERVAL_SECS));
    }

    #[test]
    fn a_missing_file_reads_as_no_capture_recorded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let activity = at_path(dir.path().join("nowhere").join("activity.json"));

        assert_eq!(activity.last_capture_at(), None);
        assert_eq!(
            idle_days(activity.last_capture_at(), Some(NOW - SECS_PER_DAY), NOW),
            Some(1)
        );
    }

    /// Corrupt bookkeeping degrades to "no capture recorded". It must never be
    /// a reason the daemon fails to start, and it must never produce a bogus
    /// idleness figure that shows the user an advisory out of nowhere.
    #[test]
    fn a_corrupt_file_reads_as_no_capture_recorded() {
        let dir = tempfile::tempdir().expect("tempdir");

        for (name, contents) in [
            ("truncated.json", "{\"last_capture_at\":"),
            ("garbage.json", "not json at all"),
            ("empty.json", ""),
            ("wrong_type.json", "{\"last_capture_at\":\"yesterday\"}"),
            ("negative.json", "{\"last_capture_at\":-5}"),
            ("zeroed.json", "{\"last_capture_at\":0}"),
            ("unrelated_shape.json", "[1,2,3]"),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, contents).expect("write");

            let activity = at_path(path.clone());
            assert_eq!(
                activity.last_capture_at(),
                None,
                "{name} should read as no record"
            );

            // And it recovers: the next capture overwrites the bad file.
            activity.record_at(NOW);
            assert_eq!(
                read_persisted(&path),
                Some(NOW),
                "{name} should be repaired"
            );
        }
    }

    /// No home directory to write to is not a failure either — the run still
    /// tracks captures, it just cannot carry them across a restart.
    #[test]
    fn no_writable_path_still_tracks_in_memory() {
        let activity = CaptureActivity::open(None);

        activity.record_at(NOW);

        assert_eq!(activity.last_capture_at(), Some(NOW));
    }

    /// Recording before `install` must not write anything, which is what keeps
    /// the rest of the test suite out of the developer's real `~/.lumen`.
    #[test]
    fn the_global_is_inert_until_installed() {
        // This test deliberately never calls `install`. If some other test in
        // the binary has, `last_capture_at` may legitimately be Some — so all
        // this can assert is that calling it does not panic.
        record_capture();
        let _ = last_capture_at();
    }
}
