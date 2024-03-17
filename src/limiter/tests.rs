use std::{
    cmp::Ordering,
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::limiter::Response;

use super::{Limit, LimiterClock, LimiterResult, LimiterState, Request};

#[test]
fn limiter_state_monotonic_after_epoch() {
    let clock = LimiterClock::new();
    let timestamp = clock.epoch + Duration::from_secs(5);

    let state = LimiterState::from_monotonic(&clock, timestamp);

    assert_eq!(state.to_monotonic(&clock), Some(timestamp));
}

#[test]
fn limiter_state_monotonic_before_epoch() {
    let clock = LimiterClock::new();
    let timestamp = clock.epoch - Duration::from_secs(5);

    let state = LimiterState::from_monotonic(&clock, timestamp);

    assert_eq!(state.to_monotonic(&clock), None);
}

#[test]
fn limiter_state_synced_wallclock_after_epoch() {
    let clock = LimiterClock::new();
    let timestamp = clock.epoch + Duration::from_secs(5);

    let mut state = LimiterState::from_monotonic(&clock, timestamp);
    state.uuid = Uuid::default();

    assert_ne!(clock.uuid, state.uuid);

    let resolved_timestamp = state.to_monotonic(&clock).unwrap();

    let difference = match timestamp.cmp(&resolved_timestamp) {
        Ordering::Equal => Duration::from_secs(0),
        Ordering::Greater => timestamp - resolved_timestamp,
        Ordering::Less => resolved_timestamp - timestamp,
    };

    assert!(difference < Duration::from_millis(100))
}

#[test]
fn limiter_state_forwards_wallclock_after_epoch() {
    let clock = LimiterClock::new();
    let timestamp = clock.epoch + Duration::from_secs(5);

    let mut state = LimiterState::from_monotonic(&clock, timestamp);
    state.uuid = Uuid::default();
    state.epoch = Some(state.epoch.unwrap() + Duration::from_secs(2));

    assert_ne!(clock.uuid, state.uuid);

    let resolved_timestamp = state.to_monotonic(&clock).unwrap() - Duration::from_secs(2);

    let difference = match timestamp.cmp(&resolved_timestamp) {
        Ordering::Equal => Duration::from_secs(0),
        Ordering::Greater => timestamp - resolved_timestamp,
        Ordering::Less => resolved_timestamp - timestamp,
    };

    assert!(difference < Duration::from_millis(100))
}

#[test]
fn limiter_state_backwards_wallclock_after_epoch() {
    let clock = LimiterClock::new();

    let timestamp = clock.epoch + Duration::from_secs(5);

    let mut state = LimiterState::from_monotonic(&clock, timestamp);
    state.uuid = Uuid::default();
    state.epoch = Some(state.epoch.unwrap() - Duration::from_secs(2));

    assert_ne!(clock.uuid, state.uuid);

    let resolved_timestamp = state.to_monotonic(&clock).unwrap() + Duration::from_secs(2);

    let difference = match timestamp.cmp(&resolved_timestamp) {
        Ordering::Equal => Duration::from_secs(0),
        Ordering::Greater => timestamp - resolved_timestamp,
        Ordering::Less => resolved_timestamp - timestamp,
    };

    assert!(difference < Duration::from_millis(100))
}

#[test]
fn limiter_state_wallclock_before_epoch() {
    let clock = LimiterClock::new();

    let timestamp = clock.epoch - Duration::from_secs(5);

    let mut state = LimiterState::from_monotonic(&clock, timestamp);
    state.uuid = Uuid::default();

    assert_ne!(clock.uuid, state.uuid);

    assert_eq!(state.to_monotonic(&clock), None);
}

#[test]
fn limit_requests_without_tokens() {
    let clock = LimiterClock::new();
    let mut request_time = clock.epoch;
    let mut limit = Limit {
        count: 5,
        r#type: super::LimitItem::Request,
        period: 10,
        state: None,
    };

    let limit_request = |limit: &mut Limit, request_time: &Instant| -> LimiterResult {
        let request = Request {
            arrived_at: *request_time,
            estimated_tokens: 1,
        };
        //println!("{:?}", limit.state);

        limit.request(&clock, &request)
    };

    let limit_response = |limit: &mut Limit, request_time: &Instant| -> LimiterResult {
        let response = Response {
            request: Request {
                arrived_at: *request_time,
                estimated_tokens: 1,
            },
            actual_tokens: 1,
        };

        limit.response(&clock, &response)
    };

    for _ in 0..5 {
        assert_eq!(
            limit_request(&mut limit, &request_time),
            LimiterResult::Ready
        );
        assert_eq!(
            limit_response(&mut limit, &request_time),
            LimiterResult::Ready
        );
    }
    assert_eq!(
        limit_request(&mut limit, &request_time),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(2))
    );
    assert_eq!(
        limit_response(&mut limit, &request_time),
        LimiterResult::Ready
    );
    assert_eq!(
        limit_request(&mut limit, &request_time),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(4))
    );
    assert_eq!(
        limit_response(&mut limit, &request_time),
        LimiterResult::Ready
    );

    request_time += Duration::from_secs(6);

    assert_eq!(
        limit_request(&mut limit, &request_time),
        LimiterResult::Ready
    );
    assert_eq!(
        limit_response(&mut limit, &request_time),
        LimiterResult::Ready
    );
    assert_eq!(
        limit_request(&mut limit, &request_time),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(2))
    );
    assert_eq!(
        limit_response(&mut limit, &request_time),
        LimiterResult::Ready
    );

    request_time += Duration::from_secs(8);

    for _ in 0..3 {
        assert_eq!(
            limit_request(&mut limit, &request_time),
            LimiterResult::Ready
        );
    }
    assert_eq!(
        limit_request(&mut limit, &request_time),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(2))
    );

    request_time += Duration::from_secs(2);
    for _ in 0..3 {
        request_time += Duration::from_secs(2);
        assert_eq!(
            limit_request(&mut limit, &request_time),
            LimiterResult::Ready
        );
    }
    assert_eq!(
        limit_request(&mut limit, &request_time),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(2))
    );
}

#[test]
fn limit_requests_with_tokens_single_pass() {
    let clock = LimiterClock::new();
    let mut request_time = clock.epoch;
    let mut limit = Limit {
        count: 128,
        r#type: super::LimitItem::Token,
        period: 8,
        state: None,
    };

    let limit_request =
        |limit: &mut Limit, request_time: &Instant, estimated_tokens: u64| -> LimiterResult {
            let request = Request {
                arrived_at: *request_time,
                estimated_tokens,
            };

            limit.request(&clock, &request)
        };

    for _ in 0..8 {
        assert_eq!(
            limit_request(&mut limit, &request_time, 16),
            LimiterResult::Ready
        );
    }
    assert_eq!(
        limit_request(&mut limit, &request_time, 16),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(1))
    );
    request_time += Duration::from_secs(2);
    assert_eq!(
        limit_request(&mut limit, &request_time, 16),
        LimiterResult::Ready
    );
    assert_eq!(
        limit_request(&mut limit, &request_time, 16),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(1))
    );

    request_time += Duration::from_secs(2);
    assert_eq!(
        limit_request(&mut limit, &request_time, 8),
        LimiterResult::Ready
    );
    assert_eq!(
        limit_request(&mut limit, &request_time, 4),
        LimiterResult::Ready
    );
    assert_eq!(
        limit_request(&mut limit, &request_time, 5),
        LimiterResult::WaitUntil(request_time + Duration::from_micros(62500))
    );
    assert_eq!(
        limit_request(&mut limit, &request_time, 8),
        LimiterResult::WaitUntil(request_time + Duration::from_micros(562500))
    );
    assert_eq!(
        limit_request(&mut limit, &request_time, 129),
        LimiterResult::Oversized
    );
    request_time += Duration::from_micros(562500);

    for _ in 0..3 {
        request_time += Duration::from_millis(500);
        assert_eq!(
            limit_request(&mut limit, &request_time, 8),
            LimiterResult::Ready
        );
    }
    assert_eq!(
        limit_request(&mut limit, &request_time, 8),
        LimiterResult::WaitUntil(request_time + Duration::from_millis(500))
    );
}

/*#[test]
fn limit_requests_with_tokens_two_pass() {
    let clock = LimiterClock::new();
    let mut request_time = clock.epoch;
    let mut limit = Limit {
        count: 128,
        r#type: super::LimitItem::Token,
        period: 8,
        state: None,
    };

    let limit_request = |limit: &mut Limit,
                                  request_time: &Instant,
                                  estimated_tokens: u64,
                                  actual_tokens: u64|
     -> (LimiterResult, LimiterResult) {
        let request = Request {
            arrived_at: *request_time,
            estimated_tokens,
        };
        let response = Response {
            request: Request {
                arrived_at: *request_time,
                estimated_tokens,
            },
            actual_tokens,
        };

        (
            limit.request(&clock, &request),
            limit.response(&clock, &response),
        )
    };

    for _ in 0..8 {
        assert_eq!(
            limit_request(&mut limit, &request_time, 16),
            LimiterResult::Ready
        );
    }
    assert_eq!(
        limit_request(&mut limit, &request_time, 16),
        LimiterResult::WaitUntil(request_time + Duration::from_secs(1))
    );
}*/
