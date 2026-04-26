//! Retry policy with exponential backoff and per-origin circuit breaker.
//!
//! The [`RetryPolicy`] and [`CircuitBreaker`] are defined in
//! [`crate::client`] and used internally by [`AnmlClient`](crate::client::AnmlClient).
//! They are re-exported here for convenience.
//!
//! The retry policy handles transient HTTP failures (5xx, connection errors).
//! The circuit breaker is per-origin and prevents hammering a failing service.
//! Both are independent of ANML-level `retry-budget` (which governs flow step
//! retries in [`FlowNavigator`](crate::flow::FlowNavigator)).

pub use crate::client::{CircuitBreaker, RetryPolicy};
