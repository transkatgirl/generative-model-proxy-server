use std::{num::NonZeroU32, time::{Instant, Duration}};

use governor::{
    middleware::StateInformationMiddleware, Quota, RateLimiter,
};

use crate::api;

type StateInformationDirectRateLimiter<MW = StateInformationMiddleware> = RateLimiter<
    governor::state::direct::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
    MW,
>;

pub struct Limiter {
    requests_per_minute: StateInformationDirectRateLimiter,
    requests_per_day: StateInformationDirectRateLimiter,
    tokens_per_minute: StateInformationDirectRateLimiter,
    tokens_per_day: StateInformationDirectRateLimiter,
    quota: api::Quota,
}

pub enum RequestQuotaStatus {
    Ready(u32),
    LimitedUntil(Instant),
}

pub enum TokenQuotaStatus {
    Ready(u32),
    LimitedUntil(Instant),
    Oversized,
}

impl Limiter {
    pub fn new(quota: api::Quota) -> Self {
        Limiter {
            requests_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.requests_per_minute).unwrap_or(NonZeroU32::MAX),
            )).with_middleware::<StateInformationMiddleware>(),
            requests_per_day: RateLimiter::direct(
                Quota::with_period(Duration::new(86400, 0) / quota.requests_per_day)
                    .unwrap()
                    .allow_burst(
                        NonZeroU32::new(quota.requests_per_day).unwrap_or(NonZeroU32::MAX),
                    ),
            ).with_middleware::<StateInformationMiddleware>(),
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

    pub fn request(&self) -> RequestQuotaStatus {
        let mut capacity = u32::MAX;
        let mut earliest = Instant::now();

        match self.requests_per_minute.check() {
            Ok(snapshot) => {
                capacity = capacity.min(snapshot.remaining_burst_capacity());
            },
            Err(notuntil) => {
                capacity = 0;
                earliest = earliest.max(notuntil.earliest_possible());
            }
        }

        match self.requests_per_day.check() {
            Ok(snapshot) => {
                capacity = capacity.min(snapshot.remaining_burst_capacity());
            },
            Err(notuntil) => {
                capacity = 0;
                earliest = earliest.max(notuntil.earliest_possible());
            }
        }

        if capacity > 0 {
            RequestQuotaStatus::Ready(capacity)
        } else {
            RequestQuotaStatus::LimitedUntil(earliest)
        }
    }

    pub fn tokens(&self, tokens: u32) -> TokenQuotaStatus {
        let tokens = NonZeroU32::new(tokens).unwrap_or(NonZeroU32::MIN);
        let mut capacity = u32::MAX;
        let mut earliest = Instant::now();

        match self.tokens_per_minute.check_n(tokens) {
            Ok(check) => match check {
                Ok(snapshot) => {
                    capacity = capacity.min(snapshot.remaining_burst_capacity());
                },
                Err(notuntil) => {
                    capacity = 0;
                    earliest = earliest.max(notuntil.earliest_possible());
                },
            },
            Err(_) => {
                return TokenQuotaStatus::Oversized;
            }
        }

        match self.tokens_per_day.check_n(tokens) {
            Ok(check) => match check {
                Ok(snapshot) => {
                    capacity = capacity.min(snapshot.remaining_burst_capacity());
                },
                Err(notuntil) => {
                    capacity = 0;
                    earliest = earliest.max(notuntil.earliest_possible());
                },
            },
            Err(_) => {
                return TokenQuotaStatus::Oversized;
            }
        }

        if capacity > 0 {
            TokenQuotaStatus::Ready(capacity)
        } else {
            TokenQuotaStatus::LimitedUntil(earliest)
        }
    }

    pub fn tokens_bounded(&self, min_tokens: u32, max_tokens: u32) -> TokenQuotaStatus {
        if min_tokens > max_tokens || max_tokens > self.quota.tokens_per_minute.min(self.quota.tokens_per_day) {
            return TokenQuotaStatus::Oversized;
        }

        let additional_tokens = max_tokens - min_tokens;
        let tokens = NonZeroU32::new(min_tokens).unwrap_or(NonZeroU32::MIN);
        let mut capacity = u32::MAX;
        let mut earliest = Instant::now();

        match self.tokens_per_minute.check_n(tokens) {
            Ok(check) => match check {
                Ok(snapshot) => {
                    let static_capacity = snapshot.remaining_burst_capacity();
                    capacity = capacity.min(static_capacity);

                    if additional_tokens > static_capacity {
                        earliest = earliest.max(Instant::now().checked_add(Duration::new(60, 0).mul_f32(
                            (additional_tokens as f32 - static_capacity as f32) / self.quota.tokens_per_minute as f32,
                        )).unwrap());
                    }
                },
                Err(notuntil) => {
                    capacity = 0;
                    earliest = earliest.max(notuntil.earliest_possible());
                },
            },
            Err(_) => {
                return TokenQuotaStatus::Oversized;
            }
        }

        match self.tokens_per_day.check_n(tokens) {
            Ok(check) => match check {
                Ok(snapshot) => {
                    let static_capacity = snapshot.remaining_burst_capacity();
                    capacity = capacity.min(static_capacity);

                    if additional_tokens > static_capacity {
                        earliest = earliest.max(Instant::now().checked_add(Duration::new(86400, 0).mul_f32(
                            (additional_tokens as f32 - static_capacity as f32) / self.quota.tokens_per_day as f32,
                        )).unwrap());
                    }
                },
                Err(notuntil) => {
                    capacity = 0;
                    earliest = earliest.max(notuntil.earliest_possible());
                },
            },
            Err(_) => {
                return TokenQuotaStatus::Oversized;
            }
        }

        if capacity >= additional_tokens.max(1) {
            TokenQuotaStatus::Ready(capacity)
        } else {
            TokenQuotaStatus::LimitedUntil(earliest)
        }
    }
}
