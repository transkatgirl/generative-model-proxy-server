use std::{num::NonZeroU32, time::Duration};

use governor::{
    middleware::StateInformationMiddleware, DefaultDirectRateLimiter, Quota, RateLimiter,
};
use tokio::time;

use crate::api;

type StateInformationDirectRateLimiter<MW = StateInformationMiddleware> = RateLimiter<
    governor::state::direct::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
    MW,
>;

pub struct Limiter {
    requests_per_minute: DefaultDirectRateLimiter,
    requests_per_day: DefaultDirectRateLimiter,
    tokens_per_minute: StateInformationDirectRateLimiter,
    tokens_per_day: StateInformationDirectRateLimiter,
    quota: api::Quota,
}

pub enum BlockingQuotaStatus {
    Ready,
    OverQuota,
}

impl Limiter {
    pub fn new(quota: api::Quota) -> Self {
        Limiter {
            requests_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.requests_per_minute).unwrap_or(NonZeroU32::MAX),
            )),
            requests_per_day: RateLimiter::direct(
                Quota::with_period(Duration::new(86400, 0) / quota.requests_per_day)
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
                Quota::with_period(Duration::new(86400, 0) / quota.tokens_per_day)
                    .unwrap()
                    .allow_burst(NonZeroU32::new(quota.tokens_per_day).unwrap_or(NonZeroU32::MAX)),
            )
            .with_middleware::<StateInformationMiddleware>(),
            quota,
        }
    }

    pub async fn wait_until_request_ready(&self) {
        self.requests_per_minute.until_ready().await;
        self.requests_per_day.until_ready().await;
    }

    pub async fn wait_until_token_request_ready(
        &self,
        tokens: u32,
    ) -> BlockingQuotaStatus {
        if tokens > self.quota.tokens_per_minute
            || tokens > self.quota.tokens_per_day / 24
        {
            return BlockingQuotaStatus::OverQuota;
        }

        self.wait_until_request_ready().await;
        self.wait_until_tokens(tokens).await;

        BlockingQuotaStatus::Ready
    }

    pub async fn wait_until_bounded_token_request_ready(
        &self,
        token_minimum: u32,
        token_upper_bound: u32,
    ) -> BlockingQuotaStatus {
        if token_minimum > token_upper_bound {
            return BlockingQuotaStatus::OverQuota;
        }

        if token_upper_bound > self.quota.tokens_per_minute
            || token_upper_bound > self.quota.tokens_per_day / 24
        {
            return BlockingQuotaStatus::OverQuota;
        }

        self.wait_until_request_ready().await;

        let (capacity_minute, capacity_day) = self.wait_until_tokens(token_minimum).await;

        let tokens = token_upper_bound - token_minimum;

        if tokens > capacity_minute {
            let wait_time: Duration = Duration::new(60, 0).mul_f32(
                (tokens as f32 - capacity_minute as f32) / self.quota.tokens_per_minute as f32,
            ) + Duration::new(1, 0);
            time::sleep(wait_time).await;
        }

        if tokens > capacity_day {
            let wait_time: Duration = Duration::new(86400, 0)
                .mul_f32((tokens as f32 - capacity_day as f32) / self.quota.tokens_per_day as f32)
                + Duration::new(1, 0);
            time::sleep(wait_time).await;
        }

        BlockingQuotaStatus::Ready
    }

    pub async fn wait_until_tokens(&self, tokens: u32) -> (u32, u32) {
        let (mut capacity_minute, mut capacity_day) = (0, 0);

        match self
            .tokens_per_minute
            .until_n_ready(NonZeroU32::new(tokens).unwrap_or(NonZeroU32::MIN))
            .await
        {
            Ok(snapshot) => capacity_minute = snapshot.remaining_burst_capacity(),
            Err(_) => time::sleep(Duration::new(60, 0)).await,
        }

        match self
            .tokens_per_day
            .until_n_ready(NonZeroU32::new(tokens).unwrap_or(NonZeroU32::MIN))
            .await
        {
            Ok(snapshot) => capacity_day = snapshot.remaining_burst_capacity(),
            Err(_) => time::sleep(Duration::new(86400, 0)).await,
        }

        (capacity_minute, capacity_day)
    }
}
