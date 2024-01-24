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
    requests_per_hour: DefaultDirectRateLimiter,
    tokens_per_minute: StateInformationDirectRateLimiter,
    tokens_per_hour: StateInformationDirectRateLimiter,
    quota: api::Quota,
}

pub enum BlockingQuotaStatus {
    Ready,
    OverQuota,
}

impl Limiter {
    pub fn new(quota: api::Quota) -> Self {
        // TODO: Implement a proper _per_day limiter
        Limiter {
            requests_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.requests_per_minute).unwrap_or(NonZeroU32::MAX),
            )),
            requests_per_hour: RateLimiter::direct(Quota::per_hour(
                NonZeroU32::new(quota.requests_per_day / 24).unwrap_or(NonZeroU32::MAX),
            )),
            tokens_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.tokens_per_minute).unwrap_or(NonZeroU32::MAX),
            ))
            .with_middleware::<StateInformationMiddleware>(),
            tokens_per_hour: RateLimiter::direct(Quota::per_hour(
                NonZeroU32::new(quota.tokens_per_day / 24).unwrap_or(NonZeroU32::MAX),
            ))
            .with_middleware::<StateInformationMiddleware>(),
            quota,
        }
    }

    pub async fn wait_until_request_ready(&self) {
        self.requests_per_minute.until_ready().await;
        self.requests_per_hour.until_ready().await;
    }

    pub async fn wait_until_tokens_ready(
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

        self.wait_until_request_ready();

        let (capacity_minute, capacity_hour) = self.wait_until_tokens(token_minimum).await;

        if token_upper_bound > capacity_minute {
            let wait_time: Duration = Duration::new(60, 0).mul_f32(
                (token_upper_bound as f32 - capacity_minute as f32)
                    / self.quota.tokens_per_minute as f32,
            ) + Duration::new(1, 0);
            time::sleep(wait_time).await;
        }

        if token_upper_bound > capacity_hour {
            let wait_time: Duration = Duration::new(3600, 0).mul_f32(
                (token_upper_bound as f32 - capacity_hour as f32)
                    / (self.quota.tokens_per_day / 24) as f32,
            ) + Duration::new(1, 0);
            time::sleep(wait_time).await;
        }

        BlockingQuotaStatus::Ready
    }

    pub async fn wait_until_tokens(&self, tokens: u32) -> (u32, u32) {
        let (mut rem_minute, mut rem_hour) = (0, 0);

        match self
            .tokens_per_minute
            .until_n_ready(NonZeroU32::new(tokens).unwrap_or(NonZeroU32::MIN))
            .await
        {
            Ok(snapshot) => rem_minute = snapshot.remaining_burst_capacity(),
            Err(_) => time::sleep(Duration::new(60, 0)).await,
        }

        match self
            .tokens_per_hour
            .until_n_ready(NonZeroU32::new(tokens).unwrap_or(NonZeroU32::MIN))
            .await
        {
            Ok(snapshot) => rem_hour = snapshot.remaining_burst_capacity(),
            Err(_) => time::sleep(Duration::new(3600, 0)).await,
        }

        (rem_minute, rem_hour)
    }
}
