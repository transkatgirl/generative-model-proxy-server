use std::{num::NonZeroU32, sync::Arc, time::Duration};

use governor::{
    middleware::StateInformationMiddleware, DefaultDirectRateLimiter, Quota, RateLimiter,
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time,
};

use super::model::{CallableModelAPI, RoutableModelRequest, RoutableModelResponse};

#[derive(Serialize, Deserialize, Debug)]
pub(super) enum LimitItem {
    Request,
    Token,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct Limit {
    count: u32,
    item_type: LimitItem,
    per: Duration,
}

type StateInformationDirectRateLimiter<MW = StateInformationMiddleware> = RateLimiter<
    governor::state::direct::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
    MW,
>;

#[derive(Debug)]
pub(super) struct Limiter {
    request_limiters: Vec<DefaultDirectRateLimiter>,
    token_limiters: Vec<(StateInformationDirectRateLimiter, Arc<Semaphore>)>,
}

#[derive(Debug)]
pub(super) struct PendingTokenHandle {
    handles: Vec<OwnedSemaphorePermit>,
    held_tokens: u32,
}

impl Limiter {
    #[tracing::instrument(level = "debug")]
    pub(super) fn new(quota: &super::Quota) -> Self {
        let mut request_limiters = Vec::new();
        let mut token_limiters = Vec::new();

        for limit in &quota.limits {
            let count = NonZeroU32::new(limit.count).unwrap_or(NonZeroU32::MIN);

            match limit.item_type {
                LimitItem::Request =>
                {
                    #[allow(deprecated)]
                    if let Some(quota) = Quota::new(count, limit.per) {
                        request_limiters.push(RateLimiter::direct(quota));
                    }
                }
                LimitItem::Token =>
                {
                    #[allow(deprecated)]
                    if let Some(quota) = Quota::new(count, limit.per) {
                        token_limiters.push((
                            RateLimiter::direct(quota)
                                .with_middleware::<StateInformationMiddleware>(),
                            Arc::new(Semaphore::new(limit.count as usize)),
                        ));
                    }
                }
            }
        }

        Limiter {
            request_limiters,
            token_limiters,
        }
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn plain_request(&self) {
        for limiter in &self.request_limiters {
            limiter.until_ready().await;
        }
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn model_request(
        &self,
        model: impl CallableModelAPI,
        request: impl RoutableModelRequest,
    ) -> Option<PendingTokenHandle> {
        self.plain_request().await;

        let mut handles = Vec::new();
        let tokens = model.get_context_len().unwrap_or(1) * request.get_total_n();

        for (limiter, semaphore) in &self.token_limiters {
            if let Ok(barrier) = semaphore.clone().acquire_many_owned(tokens).await {
                handles.push(barrier);
            } else {
                return None;
            };

            let mut needs_capacity = true;
            while needs_capacity {
                let state = limiter.until_ready().await;
                if tokens > state.remaining_burst_capacity() {
                    time::sleep(
                        state.quota().replenish_interval()
                            * (tokens - state.remaining_burst_capacity()),
                    )
                    .await;
                } else {
                    needs_capacity = false
                }
            }
        }

        Some(PendingTokenHandle {
            handles,
            held_tokens: tokens,
        })
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn model_response(
        &self,
        response: impl RoutableModelResponse,
        handle: PendingTokenHandle,
    ) -> Option<()> {
        let tokens = NonZeroU32::new(match response.get_token_count() {
            Some(t) => t,
            None => handle.held_tokens,
        })
        .unwrap_or(NonZeroU32::MIN);

        let mut error = false;
        for (limiter, _) in &self.token_limiters {
            if limiter.until_n_ready(tokens).await.is_err() {
                error = true;
            }
        }

        match error {
            true => None,
            false => Some(()),
        }
    }
}
