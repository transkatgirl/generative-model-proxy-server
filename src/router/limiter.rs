use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

use governor::{
    middleware::StateInformationMiddleware, DefaultDirectRateLimiter, Quota, RateLimiter,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time,
};

use crate::{
    api::{self, Model},
    router::{ModelRequest, ModelResponse},
};

type StateInformationDirectRateLimiter<MW = StateInformationMiddleware> = RateLimiter<
    governor::state::direct::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
    MW,
>;

#[derive(Debug)]
pub struct Limiter {
    requests_per_minute: DefaultDirectRateLimiter,
    requests_per_day: DefaultDirectRateLimiter,
    tokens_per_minute: StateInformationDirectRateLimiter,
    tokens_per_day: StateInformationDirectRateLimiter,
    tokens_per_minute_barrier: Arc<Semaphore>,
    tokens_per_day_barrier: Arc<Semaphore>,
    quota: api::Quota,
}

#[derive(Debug)]
pub struct ModelRequestHandle {
    tokens_per_minute_handle: Option<OwnedSemaphorePermit>,
    tokens_per_day_handle: Option<OwnedSemaphorePermit>,
    request_tokens: u32,
    held_tokens: u32,
}

impl Limiter {
    #[tracing::instrument(level = "trace")]
    pub fn new(quota: api::Quota) -> Self {
        Limiter {
            requests_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.requests_per_minute).unwrap_or(NonZeroU32::MAX),
            )),
            requests_per_day: RateLimiter::direct(
                Quota::with_period(Duration::new(86400, 0))
                    .unwrap()
                    .allow_burst(
                        NonZeroU32::new(quota.requests_per_day).unwrap_or(NonZeroU32::MAX),
                    ),
            ),
            tokens_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.tokens_per_minute).unwrap_or(NonZeroU32::MAX),
            ))
            .with_middleware::<StateInformationMiddleware>(),
            tokens_per_day: RateLimiter::direct(
                Quota::with_period(Duration::new(86400, 0))
                    .unwrap()
                    .allow_burst(NonZeroU32::new(quota.tokens_per_day).unwrap_or(NonZeroU32::MAX)),
            )
            .with_middleware::<StateInformationMiddleware>(),
            tokens_per_minute_barrier: Arc::new(Semaphore::new(quota.tokens_per_minute as usize)),
            tokens_per_day_barrier: Arc::new(Semaphore::new(quota.tokens_per_day as usize)),
            quota,
        }
    }

    #[tracing::instrument(level = "debug")]
    pub fn request(&self) -> Result<(), Instant> {
        match (
            self.requests_per_minute.check(),
            self.requests_per_day.check(),
        ) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(point), Ok(())) => Err(point.earliest_possible()),
            (Ok(()), Err(point)) => Err(point.earliest_possible()),
            (Err(point_1), Err(point_2)) => {
                Err(point_1.earliest_possible().max(point_2.earliest_possible()))
            }
        }
    }

    #[tracing::instrument(level = "debug")]
    fn tokens_bounded(&self, min_tokens: u32, max_tokens: u32) -> Result<Result<(), Instant>, ()> {
        if min_tokens > max_tokens
            || max_tokens > self.quota.tokens_per_minute.min(self.quota.tokens_per_day)
        {
            return Err(());
        }

        let tokens = NonZeroU32::new(min_tokens).unwrap_or(NonZeroU32::MIN);
        let additional_tokens = max_tokens - min_tokens;

        match (
            self.tokens_per_minute.check_n(tokens),
            self.tokens_per_day.check_n(tokens),
        ) {
            (Ok(check_min), Ok(check_day)) => {
                let point_1 = match check_min {
                    Ok(snapshot) => {
                        let capacity = snapshot.remaining_burst_capacity();

                        if additional_tokens > capacity {
                            Some(
                                Instant::now()
                                    .checked_add(Duration::new(60, 0).mul_f32(
                                        (additional_tokens as f32 - capacity as f32)
                                            / self.quota.tokens_per_minute as f32,
                                    ))
                                    .unwrap(),
                            )
                        } else {
                            None
                        }
                    }
                    Err(notuntil) => Some(
                        notuntil.earliest_possible().max(
                            Instant::now()
                                .checked_add(Duration::new(60, 0).mul_f32(
                                    max_tokens as f32 / self.quota.tokens_per_minute as f32,
                                ))
                                .unwrap(),
                        ),
                    ),
                };
                let point_2 =
                    match check_day {
                        Ok(snapshot) => {
                            let capacity = snapshot.remaining_burst_capacity();

                            if additional_tokens > capacity {
                                Some(
                                    Instant::now()
                                        .checked_add(Duration::new(86400, 0).mul_f32(
                                            (additional_tokens as f32 - capacity as f32)
                                                / self.quota.tokens_per_day as f32,
                                        ))
                                        .unwrap(),
                                )
                            } else {
                                None
                            }
                        }
                        Err(notuntil) => Some(
                            notuntil.earliest_possible().max(
                                Instant::now()
                                    .checked_add(Duration::new(86400, 0).mul_f32(
                                        max_tokens as f32 / self.quota.tokens_per_day as f32,
                                    ))
                                    .unwrap(),
                            ),
                        ),
                    };

                match (point_1, point_2) {
                    (Some(point_1), Some(point_2)) => Ok(Err(point_1.max(point_2))),
                    (None, Some(point)) => Ok(Err(point)),
                    (Some(point), None) => Ok(Err(point)),
                    (None, None) => Ok(Ok(())),
                }
            }
            (Ok(_), Err(_)) => Err(()),
            (Err(_), Ok(_)) => Err(()),
            (Err(_), Err(_)) => Err(()),
        }
    }

    #[tracing::instrument(level = "debug")]
    pub async fn wait_request(&self) {
        if let Err(point) = self.request() {
            time::sleep_until(time::Instant::from_std(point)).await;
        }
    }

    #[tracing::instrument(level = "debug")]
    pub fn immediate_model_request(
        &self,
        model: &Model,
        request: &ModelRequest,
    ) -> Result<ModelRequestHandle, ()> {
        if self.request().is_err() {
            return Err(());
        };

        match request.get_token_count(model) {
            Some(tokens) => {
                let max_tokens = request.get_max_tokens(model).unwrap_or(tokens) as u32;

                let minute_barrier = self
                    .tokens_per_minute_barrier
                    .clone()
                    .try_acquire_many_owned(max_tokens);
                let day_barrier = self
                    .tokens_per_minute_barrier
                    .clone()
                    .try_acquire_many_owned(max_tokens);

                match (minute_barrier, day_barrier) {
                    (Ok(minute_barrier), Ok(day_barrier)) => {
                        match self.tokens_bounded(tokens as u32, max_tokens) {
                            Ok(Ok(_)) => Ok(ModelRequestHandle {
                                tokens_per_minute_handle: Some(minute_barrier),
                                tokens_per_day_handle: Some(day_barrier),
                                request_tokens: tokens as u32,
                                held_tokens: max_tokens,
                            }),
                            _ => Err(()),
                        }
                    }
                    _ => Err(()),
                }
            }
            None => Ok(ModelRequestHandle {
                tokens_per_minute_handle: None,
                tokens_per_day_handle: None,
                request_tokens: 0,
                held_tokens: 0,
            }),
        }
    }

    #[tracing::instrument(level = "debug")]
    pub async fn wait_model_request(
        &self,
        model: &Model,
        request: &ModelRequest,
    ) -> Result<ModelRequestHandle, ()> {
        self.wait_request().await;

        match request.get_token_count(model) {
            Some(tokens) => {
                let max_tokens = request.get_max_tokens(model).unwrap_or(tokens) as u32;

                let minute_barrier = self
                    .tokens_per_minute_barrier
                    .clone()
                    .acquire_many_owned(max_tokens)
                    .await;
                let day_barrier = self
                    .tokens_per_minute_barrier
                    .clone()
                    .acquire_many_owned(max_tokens)
                    .await;

                match (minute_barrier, day_barrier) {
                    (Ok(minute_barrier), Ok(day_barrier)) => {
                        match self.tokens_bounded(tokens as u32, max_tokens) {
                            Ok(Ok(_)) => Ok(ModelRequestHandle {
                                tokens_per_minute_handle: Some(minute_barrier),
                                tokens_per_day_handle: Some(day_barrier),
                                request_tokens: tokens as u32,
                                held_tokens: max_tokens,
                            }),
                            Ok(Err(point)) => {
                                time::sleep_until(time::Instant::from_std(point)).await;
                                Ok(ModelRequestHandle {
                                    tokens_per_minute_handle: Some(minute_barrier),
                                    tokens_per_day_handle: Some(day_barrier),
                                    request_tokens: tokens as u32,
                                    held_tokens: max_tokens,
                                })
                            }
                            _ => Err(()),
                        }
                    }
                    _ => Err(()),
                }
            }
            None => Ok(ModelRequestHandle {
                tokens_per_minute_handle: None,
                tokens_per_day_handle: None,
                request_tokens: 0,
                held_tokens: 0,
            }),
        }
    }

    #[tracing::instrument(level = "debug")]
    pub async fn model_response(
        &self,
        handle: ModelRequestHandle,
        response: &ModelResponse,
    ) -> Result<(), ()> {
        if let Some(result_tokens) = response.get_token_count() {
            if result_tokens > handle.request_tokens {
                match self.tokens_bounded(
                    result_tokens - handle.request_tokens,
                    result_tokens - handle.request_tokens,
                ) {
                    Ok(Ok(_)) => {}
                    Ok(Err(point)) => {
                        time::sleep_until(time::Instant::from_std(point)).await;
                    }
                    Err(_) => {
                        if let Ok(Err(point)) = self.tokens_bounded(
                            self.quota.tokens_per_minute,
                            self.quota.tokens_per_minute,
                        ) {
                            time::sleep_until(time::Instant::from_std(point)).await;
                        }
                        return Err(());
                    }
                }
            }
        }

        Ok(())
    }
}
