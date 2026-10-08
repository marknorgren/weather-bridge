//! Process-wide pacing of upstream request starts.
use std::{
    collections::BTreeSet,
    sync::{Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};

/// Token bucket in GCRA form: up to `burst` request starts proceed at once, then one per
/// `interval`. The lock is held only while checking and consuming available capacity.
///
/// When callers must wait, the one with the lowest `priority` (the order its weather lookup
/// began in) starts first, so lookups already under way finish before newer ones take their
/// starts. A caller that cannot start by its `start_by` time is refused at once with
/// [`Saturated`], so a backlog the pace cannot clear becomes a fast refusal rather than a
/// late timeout, and the request rate never rises. A caller with capacity available now
/// starts whatever its `start_by`; the limit applies only to waiting.
pub(super) struct Limiter {
    state: Mutex<State>,
    interval: Duration,
    tolerance: Duration,
    /// Wakes waiters to check their turn again when a caller arrives, starts or leaves.
    changed: Notify,
}
struct State {
    next: Instant,
    /// Callers waiting for a start, lowest priority first; ties go in arrival order.
    waiting: BTreeSet<(u64, u64)>,
    arrivals: u64,
}
impl Limiter {
    pub fn new(burst: u32, interval: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                next: Instant::now(),
                waiting: BTreeSet::new(),
                arrivals: 0,
            }),
            interval,
            tolerance: interval * burst.max(1).saturating_sub(1),
            changed: Notify::new(),
        }
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Wait for a turn, then consume one start. A canceled or refused caller uses nothing.
    pub async fn acquire(&self, priority: u64, start_by: Instant) -> Result<(), Saturated> {
        let place = {
            let mut state = self.state();
            state.arrivals += 1;
            let place = (priority, state.arrivals);
            state.waiting.insert(place);
            place
        };
        // An older arrival can push newer waiters past their `start_by`.
        self.changed.notify_waiters();
        let _leave = Leave {
            limiter: self,
            place,
        };
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let wake_at = {
                let mut state = self.state();
                let now = Instant::now();
                let ahead = state.waiting.range(..place).count();
                let starts_at = self.start_after(state.next, now, ahead);
                if ahead == 0 && starts_at <= now {
                    state.next = state.next.max(now) + self.interval;
                    return Ok(());
                }
                if starts_at > now && starts_at > start_by {
                    return Err(Saturated);
                }
                // Earlier callers wake this one as they start or leave.
                (ahead == 0).then_some(starts_at)
            };
            match wake_at {
                Some(at) => tokio::select! {
                    () = tokio::time::sleep_until(at) => {}
                    () = changed => {}
                },
                None => changed.await,
            }
        }
    }
    /// When a caller can start if the `ahead` callers before it each start as soon as they
    /// may, with the bucket state `next` at `now`.
    fn start_after(&self, mut next: Instant, now: Instant, ahead: usize) -> Instant {
        let mut at = now;
        for _ in 0..ahead {
            at = at.max(next.checked_sub(self.tolerance).unwrap_or(at));
            next = next.max(at) + self.interval;
        }
        at.max(next.checked_sub(self.tolerance).unwrap_or(at))
    }
}
/// Removes a caller from the waiting set however `acquire` ends, and lets the others check
/// their turn again.
struct Leave<'a> {
    limiter: &'a Limiter,
    place: (u64, u64),
}
impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.limiter.state().waiting.remove(&self.place);
        self.limiter.changed.notify_waiters();
    }
}
/// The caller could not start by its `start_by` time.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Saturated;
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    impl Limiter {
        /// A start for a caller with no time limit, queued by arrival.
        async fn acquire_any_time(&self) {
            let far = Instant::now() + Duration::from_secs(3600);
            self.acquire(0, far).await.unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_after_the_limit_is_refused_at_once_without_using_capacity() {
        let interval = Duration::from_secs(1);
        let limiter = Limiter::new(2, interval);
        let started = Instant::now();
        for _ in 0..2 {
            limiter.acquire(0, started).await.unwrap();
        }
        // Capacity frees up after one interval: too late for a caller that must start
        // within half of one, so it is refused at once.
        assert_eq!(
            limiter.acquire(0, started + interval / 2).await,
            Err(Saturated)
        );
        assert_eq!(started.elapsed(), Duration::ZERO);
        limiter.acquire(0, started + interval).await.unwrap();
        assert_eq!(started.elapsed(), interval, "the refusal reserved nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn available_capacity_is_used_even_after_the_start_limit() {
        let limiter = Limiter::new(1, Duration::from_secs(1));
        let long_ago = Instant::now();
        tokio::time::advance(Duration::from_secs(60)).await;
        limiter.acquire(0, long_ago).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_oldest_lookup_starts_first_and_pushes_newer_ones_past_their_limit() {
        let interval = Duration::from_secs(1);
        let limiter = Arc::new(Limiter::new(1, interval));
        let t0 = Instant::now();
        limiter.acquire_any_time().await;
        tokio::time::advance(Duration::from_millis(10)).await;

        // A newer lookup waits for the next start, due at one interval.
        let newer = {
            let limiter = limiter.clone();
            tokio::spawn(async move { limiter.acquire(2, t0 + interval).await })
        };
        tokio::task::yield_now().await;
        assert!(!newer.is_finished(), "the newer lookup waits for capacity");

        // An older lookup arrives later but goes first, so the newer one cannot start by
        // its limit and is refused at once.
        let older = {
            let limiter = limiter.clone();
            tokio::spawn(async move { limiter.acquire(1, t0 + interval * 5).await })
        };
        assert_eq!(newer.await.unwrap(), Err(Saturated));
        assert!(Instant::now() < t0 + interval);
        assert_eq!(older.await.unwrap(), Ok(()));
        assert_eq!(Instant::now(), t0 + interval);
    }

    #[tokio::test(start_paused = true)]
    async fn limiter_allows_a_burst_then_paces_starts() {
        let interval = Duration::from_millis(200);
        let limiter = Arc::new(Limiter::new(3, interval));
        let started = Instant::now();
        let tasks: Vec<_> = (0..5)
            .map(|_| {
                let limiter = limiter.clone();
                tokio::spawn(async move {
                    limiter.acquire_any_time().await;
                    started.elapsed()
                })
            })
            .collect();

        tokio::task::yield_now().await;
        tokio::time::advance(interval).await;
        tokio::task::yield_now().await;
        tokio::time::advance(interval).await;

        let mut starts = Vec::new();
        for task in tasks {
            starts.push(task.await.unwrap());
        }
        starts.sort();
        assert_eq!(
            starts,
            [
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
                interval,
                interval * 2
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_waiters_do_not_consume_capacity() {
        let interval = Duration::from_millis(100);
        let limiter = Arc::new(Limiter::new(1, interval));
        limiter.acquire_any_time().await;

        let waiters: Vec<_> = (0..20)
            .map(|_| {
                let limiter = limiter.clone();
                tokio::spawn(async move { limiter.acquire_any_time().await })
            })
            .collect();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1)).await;
        for waiter in waiters {
            waiter.abort();
        }
        tokio::task::yield_now().await;

        let next = {
            let limiter = limiter.clone();
            tokio::spawn(async move { limiter.acquire_any_time().await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(interval - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        assert!(
            next.is_finished(),
            "canceled waits must not reserve future slots"
        );
        next.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn idle_time_refills_the_burst() {
        let interval = Duration::from_millis(100);
        let limiter = Limiter::new(3, interval);
        limiter.acquire_any_time().await;

        tokio::time::advance(interval * 10).await;

        let started = Instant::now();
        for _ in 0..3 {
            limiter.acquire_any_time().await;
        }
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
}
