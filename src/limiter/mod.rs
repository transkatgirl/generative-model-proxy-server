use std::{
    cmp::Ordering,
    ops::Add,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gcra::{GcraError, GcraState, RateLimit};
use serde::{Deserialize, Serialize};
use tracing::{event, Level};
use uuid::Uuid;

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
    state: Option<LimiterState>,
}

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
    wallclock: Option<Duration>,
    monotonic: Option<Duration>,
}

impl LimiterState {
    #[tracing::instrument(skip(clock), level = "trace", ret)]
    fn from_monotonic(clock: &LimiterClock, timestamp: Instant) -> LimiterState {
        LimiterState {
            uuid: clock.uuid,
            wallclock: SystemTime::now().duration_since(UNIX_EPOCH).ok(),
            monotonic: timestamp.checked_duration_since(clock.epoch),
        }
    }

    #[tracing::instrument(skip(clock), level = "trace", ret)]
    fn to_monotonic(&self, clock: &LimiterClock) -> Instant {
        match self.uuid == clock.uuid {
            true => match self.monotonic {
                Some(monotonic) => clock.epoch.add(monotonic),
                None => Instant::now(),
            },
            false => {
                match self
                    .wallclock
                    .and_then(|duration| UNIX_EPOCH.checked_add(duration))
                    .and_then(|timestamp| timestamp.elapsed().ok())
                {
                    Some(elapsed) => Instant::now()
                        .checked_sub(elapsed)
                        .unwrap_or(Instant::now()),
                    None => Instant::now(),
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct Request {
    pub(super) arrived_at: Instant,
    pub(super) estimated_tokens: u32,
}

#[derive(Debug)]
pub(super) struct Response {
    pub(super) request: Request,
    pub(super) actual_tokens: u32,
}

#[derive(Debug)]
pub(super) enum LimiterResult {
    Ready,
    WaitUntil(Instant),
    Oversized,
}

impl Limit {
    #[tracing::instrument(skip(clock), level = "debug", ret)]
    pub(super) fn request(&mut self, clock: &LimiterClock, request: &Request) -> LimiterResult {
        let mut state = GcraState {
            tat: self.state.map(|state| state.to_monotonic(clock)),
        };
        let rate_limit = RateLimit::new(self.count, self.per);
        let cost = match self.r#type {
            LimitItem::Request => 1,
            LimitItem::Token => request.estimated_tokens,
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

    #[tracing::instrument(skip(clock), level = "debug", ret)]
    pub(super) fn response(&mut self, clock: &LimiterClock, response: &Response) -> LimiterResult {
        if let LimitItem::Request = self.r#type {
            return LimiterResult::Ready;
        }

        let mut state = GcraState {
            tat: self.state.map(|state| state.to_monotonic(clock)),
        };
        let rate_limit = RateLimit::new(self.count, self.per);

        let result = match response
            .request
            .estimated_tokens
            .cmp(&response.actual_tokens)
        {
            Ordering::Greater => {
                let _ = state.revert_at(
                    &rate_limit,
                    response.request.arrived_at,
                    response.request.estimated_tokens - response.actual_tokens,
                );

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
                let cost = response.actual_tokens - response.request.estimated_tokens;

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
