//! Cache freshness: how long cached upstream documents stay fresh, and the per-response
//! record of the oldest source used, which bounds HTTP caching of that response.
use std::{cell::Cell, time::Duration};
use tokio::time::Instant;

/// Successful forecast, observation and alert responses stay fresh this long.
pub const CACHE_SECONDS: u64 = 120;
/// /points maps coordinates to an NWS grid and rarely changes, so it is cached much longer.
pub(super) const POINTS_CACHE_SECONDS: u64 = 6 * 60 * 60;

/// A cached upstream document and the instant its freshness ends.
#[derive(Clone)]
pub(super) struct Cached<T> {
    pub(super) value: T,
    pub(super) expires_at: Instant,
}
impl<T> Cached<T> {
    pub(super) fn fresh(value: T, seconds: u64) -> Self {
        Self {
            value,
            expires_at: Instant::now() + Duration::from_secs(seconds),
        }
    }
}
tokio::task_local! {
    static RESPONSE_EXPIRY: Cell<Instant>;
}
/// Record that the current response used a source fresh until `expires_at`. Calls outside
/// `with_cache_lifetime` (CLI, MCP) are ignored.
pub(super) fn source_expiry(expires_at: Instant) {
    let _ = RESPONSE_EXPIRY.try_with(|expiry| expiry.set(expiry.get().min(expires_at)));
}
/// Run a weather operation and return the seconds until its oldest upstream source goes
/// stale. HTTP caches may serve the response only that long, so cached data never exceeds
/// the in-process freshness promise. A failed source yields 0.
pub async fn with_cache_lifetime<T>(operation: impl Future<Output = T>) -> (T, u64) {
    RESPONSE_EXPIRY
        .scope(
            Cell::new(Instant::now() + Duration::from_secs(CACHE_SECONDS)),
            async {
                let result = operation.await;
                let seconds = RESPONSE_EXPIRY.with(|expiry| {
                    expiry
                        .get()
                        .saturating_duration_since(Instant::now())
                        .as_secs()
                });
                (result, seconds)
            },
        )
        .await
}
