//! The client-side write limiter: what keeps a bulk edit from being refused.
//!
//! Asana rate-limits three ways at once, and a bulk edit can trip all three.
//! There is a per-minute request quota (150 on a free domain, 1500 on a paid
//! one), a hard ceiling of 15 *concurrent* writes, and a cost-based quota on
//! top. Batching does not help with any of them: the documented rule is that
//! `/batch` "counts against both the standard rate limiter and the concurrent
//! request limiter as though you had made a separate HTTP request for every
//! individual action". Ten actions in one request is ten requests as far as
//! the limiters are concerned.
//!
//! So the budget here is counted in **actions, not requests**. A chunk of ten
//! costs ten, a chunk of three costs three, and a start-date edit costs one
//! extra for the read it cannot avoid (see `HttpAsanaClient::current_due`).
//! Two things are enforced:
//!
//! 1. **Concurrency.** At most [`MAX_CONCURRENT_WRITE_ACTIONS`] actions may be
//!    in flight at once, which is what makes a worker pool safe to widen: the
//!    permits, not the number of threads, are the real limit.
//! 2. **Rate.** A token bucket paced at [`WRITE_ACTIONS_PER_MINUTE`], with a
//!    full minute's worth of burst. The burst is the point — selecting forty
//!    rows and pressing `d` should go out at once, and only a *second* bulk
//!    edit on the heels of the first should have to wait.
//!
//! [`TokenBucket`] is pure: it is handed the current instant and answers how
//! long the caller must wait. All the arithmetic is therefore testable without
//! a clock and without sleeping. [`WriteThrottle`] is the thin shell that does
//! the actual waiting.

use std::{
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

/// Most write actions allowed in flight at once.
///
/// Asana's ceiling is 15. Three are left spare so an interactive write — the
/// `enter` that creates a task, the `J` that moves one between sections — is
/// not queued behind a bulk edit that has taken the whole budget.
pub const MAX_CONCURRENT_WRITE_ACTIONS: usize = 12;

/// Write actions per minute, sustained.
///
/// Under the 150/minute a free domain gets, because the quota is shared with
/// the reads that load the table: a bulk edit that used the whole allowance
/// would be followed by a refresh that could not run.
pub const WRITE_ACTIONS_PER_MINUTE: f64 = 120.0;

/// A rate limit, as tokens that accrue over time and are spent on actions.
///
/// Pure. [`TokenBucket::take`] is given the current instant and answers the
/// delay before the cost can be met, which is what lets every rule below be
/// tested by handing it arithmetic rather than by waiting for a clock.
#[derive(Clone, Debug)]
pub struct TokenBucket {
    /// Tokens that accrue per second.
    per_second: f64,
    /// The most that can be saved up, which is the size of a burst.
    capacity: f64,
    /// Tokens in hand as of `last`.
    tokens: f64,
    /// When `tokens` was last brought up to date.
    last: Instant,
}

impl TokenBucket {
    /// A bucket paced at `per_minute`, holding a minute's burst, starting full.
    ///
    /// Starting full rather than empty: the first bulk edit of a session has
    /// not used any quota, and making it wait for tokens it already has would
    /// be a limiter inventing a limit.
    pub fn per_minute(per_minute: f64, now: Instant) -> Self {
        Self {
            per_second: per_minute / 60.0,
            capacity: per_minute,
            tokens: per_minute,
            last: now,
        }
    }

    /// Spends `cost` tokens, answering how long the caller must wait first.
    ///
    /// The tokens are spent either way — the bucket goes negative rather than
    /// refusing — so a long queue is paced correctly instead of every waiter
    /// computing the same delay and then all going at once. `Duration::ZERO`
    /// means there is budget in hand.
    ///
    /// A cost above `capacity` is clamped by the arithmetic rather than
    /// rejected: it waits a full bucket's worth and then goes. Refusing it
    /// would mean a single write could be impossible, which is worse than
    /// slow.
    pub fn take(&mut self, cost: usize, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_second).min(self.capacity);
        self.last = now;
        self.tokens -= cost as f64;

        if self.tokens >= 0.0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(-self.tokens / self.per_second)
    }
}

/// The write budget, shared by every thread that sends one.
///
/// Held behind an `Arc` by the HTTP client, so cloning the client — which the
/// worker pool does per thread — shares one budget rather than handing each
/// worker its own.
#[derive(Debug)]
pub struct WriteThrottle {
    bucket: Mutex<TokenBucket>,
    /// Actions in flight, and the permits left.
    ///
    /// Separate from the bucket so a thread waiting for a permit is not
    /// holding the lock the bucket needs.
    permits: Mutex<usize>,
    released: Condvar,
    /// The ceiling `permits` counts down from.
    capacity: usize,
}

impl Default for WriteThrottle {
    fn default() -> Self {
        Self::new(MAX_CONCURRENT_WRITE_ACTIONS, WRITE_ACTIONS_PER_MINUTE)
    }
}

impl WriteThrottle {
    pub fn new(concurrent_actions: usize, per_minute: f64) -> Self {
        let capacity = concurrent_actions.max(1);
        Self {
            bucket: Mutex::new(TokenBucket::per_minute(per_minute, Instant::now())),
            permits: Mutex::new(capacity),
            released: Condvar::new(),
            capacity,
        }
    }

    /// Blocks until `cost` actions may be sent, then holds the budget.
    ///
    /// The rate is waited out *before* the permits are taken, in that order on
    /// purpose: a thread that held permits while sleeping off a rate delay
    /// would starve every other thread of concurrency it was not using.
    ///
    /// The returned guard gives the permits back when it is dropped, so a
    /// panicking or early-returning caller cannot leak the budget.
    pub fn acquire(&self, cost: usize) -> ThrottleGuard<'_> {
        let cost = cost.max(1);

        let delay = {
            let mut bucket = self.bucket.lock().expect("throttle bucket");
            bucket.take(cost, Instant::now())
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }

        // Clamped to the ceiling: a chunk larger than the whole budget would
        // otherwise wait for permits that can never all exist at once.
        let wanted = cost.min(self.capacity);
        let mut permits = self.permits.lock().expect("throttle permits");
        while *permits < wanted {
            permits = self.released.wait(permits).expect("throttle permits");
        }
        *permits -= wanted;

        ThrottleGuard {
            throttle: self,
            held: wanted,
        }
    }
}

/// The budget one in-flight chunk is holding.
#[derive(Debug)]
pub struct ThrottleGuard<'a> {
    throttle: &'a WriteThrottle,
    held: usize,
}

impl Drop for ThrottleGuard<'_> {
    fn drop(&mut self) {
        let mut permits = self
            .throttle
            .permits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *permits += self.held;
        // All, not one: a waiter wanting ten permits must not sleep through a
        // release of ten because a waiter wanting one was woken instead.
        self.throttle.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenBucket, WriteThrottle};
    use std::{
        sync::{atomic::{AtomicUsize, Ordering}, Arc, Barrier},
        time::{Duration, Instant},
    };

    #[test]
    fn a_full_bucket_lets_a_burst_through_without_waiting() {
        let now = Instant::now();
        let mut bucket = TokenBucket::per_minute(120.0, now);

        // Forty rows and a keypress: the first bulk edit of a session has used
        // no quota, so none of it waits.
        assert_eq!(bucket.take(40, now), Duration::ZERO);
        assert_eq!(bucket.take(40, now), Duration::ZERO);
        assert_eq!(bucket.take(40, now), Duration::ZERO);
    }

    #[test]
    fn spending_past_the_burst_waits_for_the_refill() {
        let now = Instant::now();
        let mut bucket = TokenBucket::per_minute(120.0, now);

        assert_eq!(bucket.take(120, now), Duration::ZERO, "exactly the burst");
        // 120/minute is two per second, so the next two cost one second.
        assert_eq!(bucket.take(2, now), Duration::from_secs(1));
    }

    #[test]
    fn each_waiter_is_told_a_later_time_than_the_last() {
        let now = Instant::now();
        let mut bucket = TokenBucket::per_minute(60.0, now);
        bucket.take(60, now);

        // The bucket goes negative rather than refusing, which is what makes
        // the queue pace itself: three waiters get three different delays
        // instead of all being told "one second" and then going at once.
        let first = bucket.take(1, now);
        let second = bucket.take(1, now);
        let third = bucket.take(1, now);

        assert_eq!(first, Duration::from_secs(1));
        assert_eq!(second, Duration::from_secs(2));
        assert_eq!(third, Duration::from_secs(3));
    }

    #[test]
    fn tokens_accrue_while_nothing_is_spent_but_never_past_the_burst() {
        let start = Instant::now();
        let mut bucket = TokenBucket::per_minute(60.0, start);
        bucket.take(60, start);

        // Ten seconds of a one-per-second refill is ten actions in hand.
        let later = start + Duration::from_secs(10);
        assert_eq!(bucket.take(10, later), Duration::ZERO);

        // An hour of idling still only buys one minute's worth.
        let much_later = later + Duration::from_secs(3600);
        assert_eq!(bucket.take(60, much_later), Duration::ZERO);
        assert_eq!(bucket.take(1, much_later), Duration::from_secs(1));
    }

    #[test]
    fn a_cost_above_the_whole_budget_waits_rather_than_being_refused() {
        let now = Instant::now();
        let mut bucket = TokenBucket::per_minute(60.0, now);

        assert_eq!(bucket.take(90, now), Duration::from_secs(30));
    }

    #[test]
    fn concurrency_is_capped_in_actions_and_released_on_drop() {
        // Generous rate, so only the permits can block: four permits against
        // three chunks of two means the third has to wait for a release.
        let throttle = Arc::new(WriteThrottle::new(4, 100_000.0));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(4));

        let threads = (0..3)
            .map(|_| {
                let throttle = Arc::clone(&throttle);
                let in_flight = Arc::clone(&in_flight);
                let peak = Arc::clone(&peak);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    let _guard = throttle.acquire(2);
                    let now = in_flight.fetch_add(2, Ordering::SeqCst) + 2;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    in_flight.fetch_sub(2, Ordering::SeqCst);
                })
            })
            .collect::<Vec<_>>();

        start.wait();
        for thread in threads {
            thread.join().expect("worker");
        }

        assert!(
            peak.load(Ordering::SeqCst) <= 4,
            "never more than the budget in flight, saw {}",
            peak.load(Ordering::SeqCst)
        );
        assert_eq!(
            *throttle.permits.lock().expect("permits"),
            4,
            "every permit came back"
        );
    }

    #[test]
    fn a_chunk_larger_than_the_budget_still_goes() {
        // Otherwise it would wait for permits that cannot all exist at once.
        let throttle = WriteThrottle::new(4, 100_000.0);
        drop(throttle.acquire(10));

        assert_eq!(*throttle.permits.lock().expect("permits"), 4);
    }
}
