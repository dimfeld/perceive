use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

/// A structure that keeps track of wall time when multiple threads are running operations that
/// should be tracked together, but may not all run continuously.
pub struct TimeTracker {
    total: AtomicU64,
    current_track: Mutex<(std::time::Instant, u64)>,
}

impl TimeTracker {
    pub fn new() -> TimeTracker {
        TimeTracker {
            total: AtomicU64::new(0),
            current_track: Mutex::new((std::time::Instant::now(), 0)),
        }
    }

    #[must_use = "The returned handle must be retained in order to track the time."]
    pub fn begin(&self) -> TimeTrackerHandle {
        let current_time = std::time::Instant::now();
        let mut current_track = self.current_track.lock();

        if current_track.1 == 0 {
            current_track.0 = current_time;
        }

        current_track.1 += 1;

        TimeTrackerHandle { tracker: self }
    }

    fn end(&self) {
        let mut current_track = self.current_track.lock();
        let start_time = current_track.0;
        let new_active = current_track.1 - 1;
        current_track.1 = new_active;
        drop(current_track);

        if new_active == 0 {
            let elapsed = start_time.elapsed();
            self.total
                .fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
        }
    }

    pub fn total(&self) -> std::time::Duration {
        let elapsed = self.total.load(std::sync::atomic::Ordering::Relaxed);
        std::time::Duration::from_micros(elapsed)
    }
}

impl Default for TimeTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TimeTrackerHandle<'a> {
    tracker: &'a TimeTracker,
}

impl<'a> Drop for TimeTrackerHandle<'a> {
    fn drop(&mut self) {
        self.tracker.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_thread() {
        let tracker = TimeTracker::new();

        let track = tracker.begin();
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(track);

        let total = tracker.total();
        let diff = total.as_millis() as i64 - 100;
        assert!(diff.abs() < 10);
    }

    #[test]
    fn multiple_threads() {
        const WAIT_TIME: u64 = 200;
        let start_flag = std::sync::Barrier::new(10);
        let tracker = TimeTracker::new();
        std::thread::scope(|scope| {
            for _ in 0..10 {
                scope.spawn(|| {
                    start_flag.wait();
                    let _track = tracker.begin();
                    std::thread::sleep(std::time::Duration::from_millis(200));
                });
            }
        });

        let total = tracker.total();

        let diff = total.as_millis() as i64 - WAIT_TIME as i64;
        assert!(diff.abs() < 10);
    }
}
