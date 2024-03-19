use std::{
    cmp::Ordering,
    time::{Duration, Instant},
};

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use uuid::Uuid;

use crate::limiter::Response;

use super::{Limit, LimiterClock, LimiterResult, LimiterState, Request};

fn get_random_unsigned(min: u64, max: u64) -> u64 {
    SmallRng::from_entropy().gen_range(min..max)
}

fn get_random_signed(min: i64, max: i64) -> i64 {
    SmallRng::from_entropy().gen_range(min..max)
}

fn test_limiter_state_monotonic(offset: i64) {
    let clock = LimiterClock::new();
    let timestamp = if offset.is_positive() {
        clock.epoch + Duration::from_secs(offset as u64)
    } else {
        clock.epoch - Duration::from_secs(offset.unsigned_abs())
    };

    let state = LimiterState::from_monotonic(&clock, timestamp);

    if offset.is_negative() {
        assert_eq!(state.to_monotonic(&clock), None);
    } else {
        assert_eq!(state.to_monotonic(&clock), Some(timestamp));
    }
}

#[test]
fn limiter_state_monotonic_conversion() {
    test_limiter_state_monotonic(get_random_signed(2, 128));
    test_limiter_state_monotonic(0);
    test_limiter_state_monotonic(get_random_signed(-128, -2));
}

fn is_within_error(one: Instant, two: Instant) -> bool {
    const MAX_WALL_CLOCK_ERROR: Duration = Duration::from_millis(100);

    let offset = match one.cmp(&two) {
        Ordering::Equal => Duration::from_secs(0),
        Ordering::Greater => one - two,
        Ordering::Less => two - one,
    };

    offset < MAX_WALL_CLOCK_ERROR
}

fn test_limiter_state_wallclock(state_offset: i64, wallclock_offset: i64) {
    let clock = LimiterClock::new();
    let timestamp: Instant = if state_offset.is_positive() {
        clock.epoch + Duration::from_secs(state_offset as u64)
    } else {
        clock.epoch - Duration::from_secs(state_offset.unsigned_abs())
    };

    let mut state = LimiterState::from_monotonic(&clock, timestamp);
    state.uuid = Uuid::default();

    assert_ne!(clock.uuid, state.uuid);

    state.epoch = Some(if wallclock_offset.is_positive() {
        state.epoch.unwrap() + Duration::from_secs(wallclock_offset as u64)
    } else {
        state.epoch.unwrap() - Duration::from_secs(wallclock_offset.unsigned_abs())
    });

    if state_offset.is_negative() {
        assert_eq!(state.to_monotonic(&clock), None);
    } else {
        let resolved_timestamp = if wallclock_offset.is_positive() {
            state.to_monotonic(&clock).unwrap() - Duration::from_secs(wallclock_offset as u64)
        } else {
            state.to_monotonic(&clock).unwrap()
                + Duration::from_secs(wallclock_offset.unsigned_abs())
        };
        assert!(is_within_error(timestamp, resolved_timestamp));
    }
}

#[test]
fn limiter_state_wallclock_conversion() {
    let test_wallclock_offset = |offset: i64| {
        test_limiter_state_wallclock(get_random_signed(2, 128), offset);
        test_limiter_state_wallclock(0, offset);
        test_limiter_state_wallclock(get_random_signed(-128, -2), offset);
    };

    test_wallclock_offset(0);
    test_wallclock_offset(get_random_signed(2, 128));
    test_wallclock_offset(get_random_signed(-128, -2));
}

fn test_limiter_request_tokenless(
    clock: &LimiterClock,
    limit: &mut Limit,
    arrived_at: Instant,
    fail_count: u32,
) {
    let request = Request {
        arrived_at,
        estimated_tokens: 1,
    };

    let response = Response {
        request: Request {
            arrived_at,
            estimated_tokens: 1,
        },
        actual_tokens: 1,
    };

    let expected_result = if fail_count > 0 {
        LimiterResult::WaitUntil(
            arrived_at + ((Duration::from_secs(limit.period) / limit.count as u32) * fail_count),
        )
    } else {
        LimiterResult::Ready
    };

    assert_eq!(limit.request(clock, &request), expected_result);
    assert_eq!(limit.response(clock, &response), LimiterResult::Ready);
}

fn test_limiter_request_with_tokens(
    clock: &LimiterClock,
    limit: &mut Limit,
    tokens: (u64, u64),
    arrived_at: Instant,
    failed_tokens: (u32, u32),
) {
    let request = Request {
        arrived_at,
        estimated_tokens: tokens.0,
    };

    let response = Response {
        request: Request {
            arrived_at,
            estimated_tokens: tokens.0,
        },
        actual_tokens: tokens.1,
    };

    let expected_first_result = if tokens.0 > limit.count {
        LimiterResult::Oversized
    } else if failed_tokens.0 > 0 {
        LimiterResult::WaitUntil(
            arrived_at
                + ((Duration::from_secs(limit.period) / limit.count as u32) * failed_tokens.0),
        )
    } else {
        LimiterResult::Ready
    };

    let expected_second_result = if tokens.0 >= tokens.1 {
        LimiterResult::Ready
    } else {
        let excess_tokens = tokens.1 - tokens.0;

        if excess_tokens > limit.count {
            LimiterResult::Oversized
        } else if failed_tokens.1 > 0 {
            LimiterResult::WaitUntil(
                arrived_at
                    + ((Duration::from_secs(limit.period) / limit.count as u32)
                        * (failed_tokens.0 + failed_tokens.1)),
            )
        } else {
            LimiterResult::Ready
        }
    };

    assert_eq!(limit.request(clock, &request), expected_first_result);
    assert_eq!(limit.response(clock, &response), expected_second_result);
}

#[test]
fn limit_requests_without_tokens() {
    let clock = LimiterClock::new();
    let mut request_time = clock.epoch;
    let count = get_random_unsigned(3, 128);
    let mut limit = Limit {
        count,
        r#type: super::LimitItem::Request,
        period: count * get_random_unsigned(3, 128),
        state: None,
    };

    for _ in 0..limit.count {
        test_limiter_request_tokenless(&clock, &mut limit, request_time, 0);
    }
    let to_fail = get_random_unsigned(2, limit.count - 1);
    let to_succeed = get_random_unsigned(2, limit.count - 1);

    for count in 1..(to_fail + 1) {
        test_limiter_request_tokenless(&clock, &mut limit, request_time, count as u32);
    }
    request_time +=
        (Duration::from_secs(limit.period) / limit.count as u32) * (to_fail + to_succeed) as u32;

    for _ in 0..to_succeed {
        test_limiter_request_tokenless(&clock, &mut limit, request_time, 0);
    }
    test_limiter_request_tokenless(&clock, &mut limit, request_time, 1);

    request_time += Duration::from_secs(limit.period) / limit.count as u32;

    for _ in 0..get_random_unsigned(limit.count, limit.count * 2) {
        request_time += Duration::from_secs(limit.period) / limit.count as u32;
        test_limiter_request_tokenless(&clock, &mut limit, request_time, 0);
    }
    test_limiter_request_tokenless(&clock, &mut limit, request_time, 1);
}

#[test]
fn limit_requests_with_tokens_equal_passes() {
    let clock = LimiterClock::new();
    let mut request_time = clock.epoch;
    let count = get_random_unsigned(3, 128);
    let mut limit = Limit {
        count,
        r#type: super::LimitItem::Token,
        period: count * get_random_unsigned(3, 128),
        state: None,
    };

    let mut tokens_used = 0;
    while limit.count > tokens_used {
        let tokens_remaining = limit.count - tokens_used;
        let tokens_to_use = if tokens_remaining == 1 {
            1
        } else {
            get_random_unsigned(0, tokens_remaining)
        };
        tokens_used += tokens_to_use;

        test_limiter_request_with_tokens(
            &clock,
            &mut limit,
            (tokens_to_use, tokens_to_use),
            request_time,
            (0, 0),
        );
    }
    let tokens_to_use = get_random_unsigned(0, limit.count);
    test_limiter_request_with_tokens(
        &clock,
        &mut limit,
        (tokens_to_use, tokens_to_use),
        request_time,
        (tokens_to_use as u32, tokens_to_use as u32),
    );
    test_limiter_request_with_tokens(
        &clock,
        &mut limit,
        (count + 1, count + 1),
        request_time,
        (0, 0),
    );

    let to_fail = get_random_unsigned(2, limit.count - 1);
    let to_succeed = get_random_unsigned(2, limit.count - 1);

    request_time += (Duration::from_secs(limit.period) / limit.count as u32)
        * (tokens_to_use + to_succeed) as u32;

    let mut tokens_used = 0;
    while to_succeed > tokens_used {
        let tokens_remaining = to_succeed - tokens_used;
        let tokens_to_use = if tokens_remaining == 1 {
            1
        } else {
            get_random_unsigned(0, tokens_remaining)
        };
        tokens_used += tokens_to_use;

        test_limiter_request_with_tokens(
            &clock,
            &mut limit,
            (tokens_to_use, tokens_to_use),
            request_time,
            (0, 0),
        );
    }

    let mut tokens_used = 0;
    while to_fail > tokens_used {
        let tokens_remaining = to_fail - tokens_used;
        let tokens_to_use = if tokens_remaining == 1 {
            1
        } else {
            get_random_unsigned(0, tokens_remaining)
        };
        tokens_used += tokens_to_use;

        test_limiter_request_with_tokens(
            &clock,
            &mut limit,
            (tokens_to_use, tokens_to_use),
            request_time,
            (tokens_used as u32, tokens_used as u32),
        );
    }
}

#[test]
fn limit_requests_with_tokens_greater_first_pass() {}

#[test]
fn limit_requests_with_tokens_greater_second_pass() {}
