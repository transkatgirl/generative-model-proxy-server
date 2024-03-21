use std::{
    cmp::Ordering,
    time::{Duration, Instant, SystemTime},
};

use gcra::{GcraError, GcraState, RateLimit};
use serde::{Deserialize, Serialize};
use tracing::{event, Level};
use uuid::Uuid;

#[cfg(test)]
mod tests;

pub(super) struct LimiterClock {
    uuid: Uuid,
    epoch: Instant,
}

impl LimiterClock {
    pub(super) fn new() -> LimiterClock {
        LimiterClock {
            uuid: Uuid::new_v4(),
            epoch: Instant::now(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
struct LimiterState {
    uuid: Uuid,
    epoch: Option<SystemTime>,
    elasped: Option<Duration>,
}

impl LimiterState {
    #[tracing::instrument(skip(clock), level = "trace", ret)]
    fn from_monotonic(clock: &LimiterClock, timestamp: Instant) -> LimiterState {
        LimiterState {
            uuid: clock.uuid,
            epoch: SystemTime::now().checked_sub(clock.epoch.elapsed()),
            elasped: timestamp.checked_duration_since(clock.epoch),
        }
    }

    #[tracing::instrument(skip(clock), level = "trace", ret)]
    fn to_monotonic(&self, clock: &LimiterClock) -> Option<Instant> {
        self.elasped
            .and_then(|elapsed| match self.uuid == clock.uuid {
                true => clock.epoch.checked_add(elapsed),
                false => self.epoch.and_then(|epoch| {
                    epoch
                        .checked_add(elapsed)
                        .and_then(|absolute| match absolute.elapsed() {
                            Ok(duration) => Instant::now().checked_sub(duration),
                            Err(future_duration) => {
                                Instant::now().checked_add(future_duration.duration())
                            }
                        })
                }),
            })
    }
}

#[derive(Debug)]
pub(super) struct Request {
    pub(super) arrived_at: Instant,
    pub(super) estimated_tokens: u64,
}

#[derive(Debug)]
pub(super) struct Response {
    pub(super) request: Request,
    pub(super) actual_tokens: u64,
}

#[derive(PartialEq, Eq, Debug)]
pub(super) enum LimiterResult {
    Ready,
    WaitUntil(Instant),
    Oversized,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub(super) enum LimitItem {
    Request,
    Token,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct Limit {
    pub(super) count: u64,
    pub(super) r#type: LimitItem,
    pub(super) period: u64,
    state: Option<LimiterState>,
}

impl Limit {
    #[tracing::instrument(skip(clock), level = "trace", ret)]
    pub(super) fn request(&mut self, clock: &LimiterClock, request: &Request) -> LimiterResult {
        let mut state = GcraState {
            tat: self.state.and_then(|state| state.to_monotonic(clock)),
        };
        let rate_limit = RateLimit::new(
            self.count.min(u32::MAX as u64) as u32,
            Duration::from_secs(self.period),
        );
        let cost = match self.r#type {
            LimitItem::Request => 1,
            LimitItem::Token => request.estimated_tokens.min(u32::MAX as u64) as u32,
        };

        let result = match state.check_and_modify_at(&rate_limit, request.arrived_at, cost) {
            Ok(_) => LimiterResult::Ready,
            Err(GcraError::DeniedUntil { next_allowed_at }) => {
                state.tat = Some(next_allowed_at + rate_limit.period);

                LimiterResult::WaitUntil(next_allowed_at)
            }
            Err(GcraError::DeniedIndefinitely {
                cost: _,
                rate_limit: _,
            }) => return LimiterResult::Oversized,
        };

        self.state = state
            .tat
            .map(|timestamp| LimiterState::from_monotonic(clock, timestamp));

        result
    }

    #[tracing::instrument(skip(clock), level = "trace", ret)]
    pub(super) fn response(&mut self, clock: &LimiterClock, response: &Response) -> LimiterResult {
        if let LimitItem::Request = self.r#type {
            return LimiterResult::Ready;
        }

        let mut state = GcraState {
            tat: self.state.and_then(|state| state.to_monotonic(clock)),
        };
        let rate_limit = RateLimit::new(
            self.count.min(u32::MAX as u64) as u32,
            Duration::from_secs(self.period),
        );

        let result = match response
            .request
            .estimated_tokens
            .cmp(&response.actual_tokens)
        {
            Ordering::Greater => {
                let extra = (response.request.estimated_tokens - response.actual_tokens)
                    .min(u32::MAX as u64) as u32;
                let _ = state.revert_at(&rate_limit, response.request.arrived_at, extra);

                LimiterResult::Ready
            }
            Ordering::Equal => LimiterResult::Ready,
            Ordering::Less => {
                event!(
                    Level::WARN,
                    "Request had greater final token count ({}) than estimated maximum of {}!",
                    response.actual_tokens,
                    response.request.estimated_tokens
                );
                let cost = (response.actual_tokens - response.request.estimated_tokens)
                    .min(u32::MAX as u64) as u32;

                match state.check_and_modify_at(&rate_limit, response.request.arrived_at, cost) {
                    Ok(_) => LimiterResult::Ready,
                    Err(GcraError::DeniedUntil { next_allowed_at }) => {
                        state.tat = Some(next_allowed_at + rate_limit.period);

                        LimiterResult::WaitUntil(next_allowed_at)
                    }
                    Err(GcraError::DeniedIndefinitely {
                        cost: _,
                        rate_limit: _,
                    }) => {
                        event!(
                            Level::WARN,
                            "Request had greater final token count ({}) than rate limiter maximum of {}!",
                            response.actual_tokens,
                            rate_limit.resource_limit,
                        );
                        match state.check_and_modify_at(
                            &rate_limit,
                            response.request.arrived_at,
                            rate_limit.resource_limit,
                        ) {
                            Ok(_) => LimiterResult::Ready,
                            Err(GcraError::DeniedUntil { next_allowed_at }) => {
                                state.tat = Some(next_allowed_at + rate_limit.period);

                                LimiterResult::WaitUntil(next_allowed_at)
                            }
                            Err(GcraError::DeniedIndefinitely {
                                cost: _,
                                rate_limit: _,
                            }) => LimiterResult::Oversized,
                        }
                    }
                }
            }
        };

        self.state = state
            .tat
            .map(|timestamp| LimiterState::from_monotonic(clock, timestamp));

        result
    }
}
