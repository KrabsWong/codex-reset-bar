//! Background status monitoring, independent of whether the popover was opened.
use crate::{cache, format, model::StatusData};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    None,
    NewReset,
    Scheduled,
}

#[derive(Default)]
struct Tracker {
    seen: Option<i64>,
    latest: Option<i64>,
    scheduled: Option<i64>,
    visible: bool,
}

impl Tracker {
    fn observe(&mut self, data: &StatusData) {
        if let Some(latest) = data
            .latest_reset
            .as_ref()
            .and_then(|r| format::parse(&r.announced_at))
            .or_else(|| data.stats.last_reset_at.as_deref().and_then(format::parse))
        {
            // Delayed responses must not turn a newer event back into an old one.
            self.latest = Some(self.latest.map_or(latest, |old| old.max(latest)));
            if self.seen.is_none() {
                self.seen = self.latest;
            }
        }
        self.scheduled = data
            .scheduled_reset
            .as_ref()
            .and_then(|r| format::parse(r.scheduled_for.as_deref().unwrap_or(&r.announced_at)));
    }

    fn displayed(&mut self, data: &StatusData) {
        if let Some(shown) = data
            .latest_reset
            .as_ref()
            .and_then(|r| format::parse(&r.announced_at))
            .or_else(|| data.stats.last_reset_at.as_deref().and_then(format::parse))
        {
            self.latest = Some(self.latest.map_or(shown, |old| old.max(shown)));
            if self.visible {
                // A stale panel response must not acknowledge a newer background event.
                self.seen = Some(self.seen.map_or(shown, |old| old.max(shown)));
            }
        }
        // Only the serial background poll owns scheduled state. An older UI
        // response cannot erase or resurrect its most recent announcement.
    }

    fn badge(&self) -> Badge {
        if self
            .latest
            .zip(self.seen)
            .is_some_and(|(last, seen)| last > seen)
        {
            Badge::NewReset
        } else if self
            .scheduled
            .is_some_and(|at| self.latest.is_none_or(|last| at > last))
        {
            // An overdue announcement remains pending until a reset is observed.
            Badge::Scheduled
        } else {
            Badge::None
        }
    }
}

static TRACKER: OnceLock<Mutex<Tracker>> = OnceLock::new();
static UPDATE: OnceLock<fn(Badge)> = OnceLock::new();

fn seen_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_RESET_CACHE_PATH") {
        return Some(PathBuf::from(path).with_extension("seen.json"));
    }
    Some(
        PathBuf::from(std::env::var_os("HOME")?)
            .join("Library/Application Support/CodexResets/seen.json"),
    )
}

fn update(change: impl FnOnce(&mut Tracker)) {
    let Some(tracker) = TRACKER.get() else { return };
    let mut tracker = tracker.lock().unwrap();
    let before = tracker.seen;
    change(&mut tracker);
    if tracker.seen != before {
        if let Some(path) = seen_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_vec(&tracker.seen) {
                let temporary = path.with_extension("tmp");
                if std::fs::write(&temporary, json).is_ok() {
                    let _ = std::fs::rename(temporary, path);
                }
            }
        }
    }
    if let Some(handler) = UPDATE.get() {
        handler(tracker.badge());
    }
}

fn observe(data: &StatusData) {
    update(|tracker| tracker.observe(data));
}
pub(crate) fn displayed(data: &StatusData) {
    update(|tracker| tracker.displayed(data));
}

pub(crate) fn seen_at() -> Option<i64> {
    TRACKER.get().and_then(|tracker| tracker.lock().unwrap().seen)
}

pub fn set_visible(visible: bool) {
    update(|tracker| {
        tracker.visible = visible;
    });
}

/// Start once at app launch. Network work never blocks AppKit's main thread.
pub fn start(handler: fn(Badge)) {
    let seen = seen_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Option<i64>>(&bytes).ok())
        .flatten();
    if TRACKER
        .set(Mutex::new(Tracker {
            seen,
            ..Tracker::default()
        }))
        .is_err()
    {
        return;
    }
    let _ = UPDATE.set(handler);
    if let Some(cached) = cache::load() {
        observe(&cached.status);
    }
    std::thread::spawn(|| {
        loop {
            if let Some(data) = fetch_status() {
                observe(&data);
            }
            // Failures retain the last known indicator and retry at the same cadence.
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

fn fetch_status() -> Option<StatusData> {
    let base =
        std::env::var("CODEX_RESET_API_BASE").unwrap_or_else(|_| crate::api::SITE.to_owned());
    fetch_status_at(&base)
}

fn fetch_status_at(base: &str) -> Option<StatusData> {
    use block2::RcBlock;
    use objc2::rc::autoreleasepool;
    use objc2_foundation::{
        NSData, NSError, NSString, NSURL, NSURLRequest, NSURLRequestCachePolicy, NSURLResponse,
        NSURLSession,
    };
    autoreleasepool(|_| {
        let url = NSURL::URLWithString(&NSString::from_str(&format!("{base}/api/v1/status")))?;
        let request = NSURLRequest::requestWithURL_cachePolicy_timeoutInterval(
            &url,
            NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData,
            20.0,
        );
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let completion = RcBlock::new(
            move |data: *mut NSData, _: *mut NSURLResponse, error: *mut NSError| {
                // Foundation keeps these objects alive for the duration of the callback.
                let status = if error.is_null() && !data.is_null() {
                    let bytes = unsafe { &*data }.to_vec();
                    serde_json::from_slice::<crate::model::StatusEnvelope>(&bytes)
                        .ok()
                        .map(|x| x.data)
                } else {
                    None
                };
                let _ = sender.send(status);
            },
        );
        // The block only captures a thread-safe channel sender.
        let task = unsafe {
            NSURLSession::sharedSession()
                .dataTaskWithRequest_completionHandler(&request, &completion)
        };
        task.resume();
        let result = receiver
            .recv_timeout(Duration::from_secs(25))
            .ok()
            .flatten();
        task.cancel();
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(latest: &str, scheduled: Option<&str>) -> StatusData {
        serde_json::from_value(serde_json::json!({
            "latest_reset": {"id":latest,"reset_type":"regular","announced_at":latest,"text":"reset"},
            "scheduled_reset": scheduled.map(|at| serde_json::json!({
                "id":at,"reset_type":"regular","announced_at":at,"scheduled_for":at,"text":"soon"})),
            "stats":{"total":1}
        })).unwrap()
    }
    #[test]
    fn baseline_new_event_acknowledgment_and_restart() {
        let mut tracker = Tracker::default();
        tracker.observe(&status("2026-10-25T00:00:00Z", None));
        assert_eq!(tracker.badge(), Badge::None);
        tracker.observe(&status("2026-10-26T00:00:00Z", None));
        assert_eq!(tracker.badge(), Badge::NewReset);
        let mut restarted = Tracker {
            seen: tracker.seen,
            ..Tracker::default()
        };
        restarted.observe(&status("2026-10-26T00:00:00Z", None));
        assert_eq!(restarted.badge(), Badge::NewReset);
        restarted.visible = true;
        restarted.displayed(&status("2026-10-26T00:00:00Z", None));
        assert_eq!(restarted.badge(), Badge::None);
    }
    #[test]
    fn announcement_priority_completion_and_cancellation() {
        let mut tracker = Tracker::default();
        tracker.observe(&status(
            "2026-10-25T00:00:00Z",
            Some("2026-10-27T00:00:00Z"),
        ));
        assert_eq!(tracker.badge(), Badge::Scheduled);
        tracker.observe(&status(
            "2026-10-26T00:00:00Z",
            Some("2026-10-27T00:00:00Z"),
        ));
        assert_eq!(tracker.badge(), Badge::NewReset);
        tracker.seen = tracker.latest;
        assert_eq!(tracker.badge(), Badge::Scheduled);
        tracker.observe(&status("2026-10-26T00:00:00Z", None));
        assert_eq!(tracker.badge(), Badge::None);
        tracker.visible = true;
        tracker.displayed(&status(
            "2026-10-27T00:00:00Z",
            Some("2026-10-27T00:00:00Z"),
        ));
        assert_eq!(tracker.badge(), Badge::None);
    }
    #[test]
    fn opening_cached_record_clears_unread_but_preserves_pending_reset() {
        let old = status("2026-10-25T00:00:00Z", None);
        let cached = status(
            "2026-10-26T07:00:00Z",
            Some("2026-10-28T07:00:00Z"),
        );
        let mut tracker = Tracker::default();
        tracker.observe(&old);
        tracker.observe(&cached);
        tracker.displayed(&cached);
        assert_eq!(tracker.badge(), Badge::NewReset);

        tracker.visible = true;
        // Opening acknowledges the existing panel data without a network response.
        tracker.displayed(&cached);
        assert_eq!(tracker.badge(), Badge::Scheduled);

        tracker.observe(&status("2026-10-28T07:00:00Z", None));
        tracker.displayed(&cached);
        assert_eq!(tracker.badge(), Badge::NewReset);
    }

    #[test]
    fn visible_panel_does_not_acknowledge_undisplayed_background_event() {
        let mut tracker = Tracker {
            visible: true,
            ..Tracker::default()
        };
        let old = status("2026-10-25T00:00:00Z", None);
        let new = status("2026-10-26T00:00:00Z", None);
        tracker.observe(&old);
        tracker.observe(&new);
        tracker.displayed(&old);
        assert_eq!(tracker.badge(), Badge::NewReset);
        tracker.displayed(&new);
        assert_eq!(tracker.badge(), Badge::None);
    }

    #[test]
    fn old_cache_cannot_move_persisted_acknowledgment_backwards() {
        let seen = format::parse("2026-10-26T00:00:00Z");
        let mut tracker = Tracker {
            seen,
            visible: true,
            ..Tracker::default()
        };
        let old = status("2026-10-25T00:00:00Z", None);
        tracker.observe(&old);
        tracker.displayed(&old);
        assert_eq!(tracker.seen, seen);
        tracker.observe(&status("2026-10-26T00:00:00Z", None));
        assert_eq!(tracker.badge(), Badge::None);
    }

    #[test]
    fn native_fetch_accepts_status_and_rejects_invalid_payload() {
        use std::io::{Read, Write};
        for valid in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 4096];
                let size = stream.read(&mut request).unwrap();
                assert!(String::from_utf8_lossy(&request[..size]).contains("GET /api/v1/status"));
                let body = if valid {
                    serde_json::json!({"data":status("2026-10-26T00:00:00Z", None)}).to_string()
                } else {
                    "not json".to_owned()
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            });
            assert_eq!(fetch_status_at(&base).is_some(), valid);
            server.join().unwrap();
        }
    }

    /// Run separately: cargo test background_poll_without_ui -- --ignored --test-threads=1
    #[test]
    #[ignore = "60-second background integration test; run in isolation"]
    fn background_poll_without_ui() {
        use std::io::{Read, Write};
        static EVENTS: OnceLock<std::sync::mpsc::Sender<Badge>> = OnceLock::new();
        fn changed(badge: Badge) {
            let _ = EVENTS.get().unwrap().send(badge);
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let cache = std::env::temp_dir().join(format!("codex-monitor-{}.json", std::process::id()));
        // This ignored test runs alone; set configuration before starting the worker.
        unsafe {
            std::env::set_var("CODEX_RESET_API_BASE", base);
            std::env::set_var("CODEX_RESET_CACHE_PATH", &cache);
        }
        let server = std::thread::spawn(move || {
            for date in ["2026-10-25T00:00:00Z", "2026-10-26T00:00:00Z"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).unwrap();
                let body = serde_json::json!({"data":status(date, None)}).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        EVENTS.set(sender).unwrap();
        start(changed);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            Badge::None
        );
        let started = std::time::Instant::now();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(70)).unwrap(),
            Badge::NewReset
        );
        assert!(started.elapsed() >= Duration::from_secs(59));
        assert!(
            !cache.exists(),
            "background monitoring must not mount or overwrite the panel cache"
        );
        server.join().unwrap();
        let _ = std::fs::remove_file(cache.with_extension("seen.json"));
    }
}
