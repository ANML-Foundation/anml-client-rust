//! Knowledge exchange helpers (answer, refuse, ask, inform builders).
//!
//! This module provides convenience wrappers around the `anml` crate's
//! [`ResponseBuilder`](anml::builder::ResponseBuilder) that integrate with
//! the client's disclosure evaluation engine.
//!
//! The key function is [`build_answer`], which runs the full 7-step
//! disclosure algorithm before constructing an `<answer>` element. If
//! disclosure is denied, a `<refuse>` element is returned instead.
//!
//! For deferred asks (asks without an `action` attribute), `build_answer`
//! still evaluates disclosure but does not submit via HTTP — the caller
//! is responsible for including the answer in a subsequent document.

pub mod response;

pub use response::{
    build_answer, build_ask, build_inform, build_refuse, build_response, AnswerOutcome,
    ResponseBuilder as KnowledgeResponseBuilder,
};
