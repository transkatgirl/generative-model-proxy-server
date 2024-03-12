use std::{
    cmp::Ordering,
    ops::Add,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gcra::{GcraError, GcraState, RateLimit};
use serde::{Deserialize, Serialize};
use tokio::time;
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
    #[tracing::instrument(skip(clock), level = "trace")]
    fn from_monotonic(clock: &LimiterClock, timestamp: Instant) -> LimiterState {
        LimiterState {
            uuid: clock.uuid,
            wallclock: SystemTime::now().duration_since(UNIX_EPOCH).ok(),
            monotonic: timestamp.checked_duration_since(clock.epoch),
        }
    }

    #[tracing::instrument(skip(clock), level = "trace")]
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

impl Limit {
    #[tracing::instrument(skip(clock), level = "debug")]
    pub(super) async fn request(&mut self, clock: &LimiterClock, request: &Request) -> bool {
        let mut state = GcraState {
            tat: self.state.map(|state| state.to_monotonic(clock)),
        };
        let rate_limit = RateLimit::new(self.count, self.per);
        let cost = match self.r#type {
            LimitItem::Request => 1,
            LimitItem::Token => request.estimated_tokens,
        };

        match state.check_and_modify_at(&rate_limit, request.arrived_at, cost) {
            Ok(_) => {}
            Err(GcraError::DeniedUntil { next_allowed_at }) => {
                time::sleep_until(time::Instant::from_std(next_allowed_at)).await;
                state.tat = Some(next_allowed_at + rate_limit.period);
            }
            Err(GcraError::DeniedIndefinitely {
                cost: _,
                rate_limit: _,
            }) => {
                return false;
            }
        }

        self.state = state
            .tat
            .map(|timestamp| LimiterState::from_monotonic(clock, timestamp));

        true
    }

    #[tracing::instrument(skip(clock), level = "debug")]
    pub(super) async fn response(&mut self, clock: &LimiterClock, response: Response) {
        if let LimitItem::Request = self.r#type {
            return;
        }

        let mut state = GcraState {
            tat: self.state.map(|state| state.to_monotonic(clock)),
        };
        let rate_limit = RateLimit::new(self.count, self.per);

        match response
            .request
            .estimated_tokens
            .cmp(&response.actual_tokens)
        {
            Ordering::Greater => {
                let excess_cost = response.request.estimated_tokens - response.actual_tokens;

                let _ = state.revert_at(&rate_limit, response.request.arrived_at, excess_cost);
            }
            Ordering::Equal => {}
            Ordering::Less => {
                event!(
                    Level::WARN,
                    "Request had greater final token count ({}) than estimated maximum of {}!",
                    response.actual_tokens,
                    response.request.estimated_tokens
                );
                let cost = response.actual_tokens - response.request.estimated_tokens;

                if let Err(GcraError::DeniedUntil { next_allowed_at }) =
                    state.check_and_modify_at(&rate_limit, response.request.arrived_at, cost)
                {
                    time::sleep_until(time::Instant::from_std(next_allowed_at)).await;
                }
            }
        }

        self.state = state
            .tat
            .map(|timestamp| LimiterState::from_monotonic(clock, timestamp));
    }
}
