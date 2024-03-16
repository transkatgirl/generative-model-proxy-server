use core::time;
use std::{
    cmp::Ordering,
    ops::Add,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gcra::{GcraError, GcraState, RateLimit};
use serde::{Deserialize, Serialize};
use tracing::{event, Level};
use uuid::Uuid;

use super::{LimiterClock, LimiterState};

#[test]
fn test_limiter_state_monotonic_after_epoch() {
    let clock = LimiterClock::new();

    let timestamp = clock.epoch + Duration::from_secs(5);

    let state = LimiterState::from_monotonic(&clock, timestamp);

    assert_eq!(state.to_monotonic(&clock), Some(timestamp));
}

#[test]
fn test_limiter_state_monotonic_before_epoch() {
    let clock = LimiterClock::new();

    let timestamp = clock.epoch - Duration::from_secs(5);

    let state = LimiterState::from_monotonic(&clock, timestamp);

    assert_eq!(state.to_monotonic(&clock), None);
}

#[test]
fn test_limiter_state_synced_wallclock_after_epoch() {
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
fn test_limiter_state_forwards_wallclock_after_epoch() {
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
fn test_limiter_state_backwards_wallclock_after_epoch() {
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
fn test_limiter_state_wallclock_before_epoch() {
    let clock = LimiterClock::new();

    let timestamp = clock.epoch - Duration::from_secs(5);

    let mut state = LimiterState::from_monotonic(&clock, timestamp);
    state.uuid = Uuid::default();

    assert_ne!(clock.uuid, state.uuid);

    assert_eq!(state.to_monotonic(&clock), None);
}
