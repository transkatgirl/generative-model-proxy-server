use std::{
    cmp::Ordering,
    time::{Duration, Instant},
};

use gcra::{GcraError, GcraState, RateLimit};
use serde::{Deserialize, Serialize};
use tokio::time;
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
    #[tracing::instrument(level = "debug")]
    pub(super) async fn request(
        &self,
        clock_state: &mut Option<Instant>,
        request: &Request,
    ) -> bool {
        let mut state = GcraState { tat: *clock_state };
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

        *clock_state = state.tat;

        true
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn response(&self, clock_state: &mut Option<Instant>, response: Response) {
        if let LimitItem::Request = self.r#type {
            return;
        }

        let mut state = GcraState { tat: *clock_state };
        let rate_limit = RateLimit::new(self.count, self.per);

        match response
            .request
            .estimated_tokens
            .cmp(&response.actual_tokens)
        {
            Ordering::Greater => {
                let excess_cost = response.request.estimated_tokens - response.actual_tokens;

                let _ = state.revert_at(&rate_limit, response.request.arrived_at, excess_cost);
                *clock_state = state.tat;
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
                    *clock_state = Some(next_allowed_at + rate_limit.period);
                }
            }
        }
    }
}
