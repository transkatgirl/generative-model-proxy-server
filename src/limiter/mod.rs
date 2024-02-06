use std::{
    cmp::Ordering,
    sync::Arc,
    time::{Duration, Instant},
};

use gcra::{GcraError, GcraState, RateLimit};
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time};
use tracing::{event, Level};

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub(super) enum LimitItem {
    Request,
    Token,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct Limit {
    pub(super) count: u32,
    pub(super) r#type: LimitItem,
    pub(super) per: Duration,
}

#[derive(Debug)]
pub(super) struct Limiter {
    request_limiters: Vec<(RateLimit, Arc<Mutex<GcraState>>)>,
    token_limiters: Vec<(RateLimit, Arc<Mutex<GcraState>>)>,
}

#[derive(Debug)]
pub(super) struct PendingRequestHandle {
    arrived_at: Instant,
    tokens: u32,
}

// TODO: Add prioritization
impl Limiter {
    #[tracing::instrument(level = "debug")]
    pub(super) fn new(limits: &[Limit]) -> Self {
        let mut limiter = Limiter {
            request_limiters: Vec::new(),
            token_limiters: Vec::new(),
        };

        for limit in limits {
            let state = Arc::new(Mutex::new(GcraState::default()));
            let rate_limit = RateLimit::new(limit.count, limit.per);

            match limit.r#type {
                LimitItem::Request => {
                    limiter.request_limiters.push((rate_limit, state));
                }
                LimitItem::Token => {
                    limiter.token_limiters.push((rate_limit, state));
                }
            }
        }

        limiter
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn plain_request(&self, arrived_at: Instant) {
        for (rate_limit, state_mutex) in &self.request_limiters {
            let mut state = state_mutex.lock().await;

            match state.check_and_modify_at(rate_limit, arrived_at, 1) {
                Ok(_) => {}
                Err(GcraError::DeniedUntil { next_allowed_at }) => {
                    time::sleep_until(time::Instant::from_std(next_allowed_at)).await;
                    state.tat = Some(next_allowed_at + rate_limit.period);
                }
                Err(_) => {
                    event!(
                        Level::WARN,
                        "Request rate limiter has <1 capacity!\n{:?}",
                        rate_limit
                    );
                    time::sleep(rate_limit.period).await;
                }
            }
        }
    }

    #[tracing::instrument(level = "trace")]
    fn is_token_count_oversized(&self, tokens: u32) -> bool {
        for (rate_limit, _) in &self.token_limiters {
            if tokens > rate_limit.resource_limit {
                return true;
            }
        }

        false
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn token_request(
        &self,
        tokens: u32,
        arrived_at: Instant,
    ) -> Option<PendingRequestHandle> {
        if self.is_token_count_oversized(tokens) {
            return None;
        }

        self.plain_request(arrived_at).await;

        for (rate_limit, state_mutex) in &self.token_limiters {
            let mut state = state_mutex.lock().await;

            match state.check_and_modify_at(rate_limit, arrived_at, 1) {
                Ok(_) => {}
                Err(GcraError::DeniedUntil { next_allowed_at }) => {
                    time::sleep_until(time::Instant::from_std(next_allowed_at)).await;
                    state.tat = Some(next_allowed_at + rate_limit.period);
                }
                Err(GcraError::DeniedIndefinitely {
                    cost: _,
                    rate_limit: _,
                }) => {
                    event!(
                        Level::WARN,
                        "Token rate limiter has incorrect capacity!\n{:?}",
                        rate_limit
                    );
                    time::sleep(rate_limit.period).await;
                }
            }
        }

        Some(PendingRequestHandle { arrived_at, tokens })
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn token_request_finalize(&self, tokens: u32, handle: PendingRequestHandle) {
        match handle.tokens.cmp(&tokens) {
            Ordering::Greater => {
                let tokens = handle.tokens - tokens;

                for (rate_limit, state_mutex) in &self.token_limiters {
                    let mut state = state_mutex.lock().await;

                    let _ = state.revert_at(rate_limit, handle.arrived_at, tokens);
                }
            }
            Ordering::Equal => {}
            Ordering::Less => {
                event!(
                    Level::WARN,
                    "Request had greater final token count ({}) than estimated maximum of {}!",
                    tokens,
                    handle.tokens
                );
                let tokens = tokens - handle.tokens;

                let _ = self.token_request(tokens, Instant::now()).await;
            }
        }
    }
}
