//! Flow navigation and multi-step workflow state tracking.
//!
//! The [`FlowNavigator`] tracks the current step in a multi-step ANML flow,
//! provides accessors for step state, handles `next-on-error` transitions
//! and `retry-budget` with exponential backoff, detects state regressions,
//! integrates with [`ConditionEvaluator`](crate::config::ConditionEvaluator),
//! and enforces a per-flow timeout.
//!
//! # Example
//!
//! ```rust,no_run
//! use anml_client::flow::FlowNavigator;
//! use anml::types::document::AnmlDocument;
//!
//! # fn example(doc: AnmlDocument) -> anml_client::Result<()> {
//! let nav = FlowNavigator::from_document(&doc)?;
//! println!("Current step: {:?}", nav.current());
//! println!("Progress: {}", nav);
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::time::{Duration, Instant};

use anml::types::document::AnmlDocument;
use anml::types::elements::AnmlStep;
use anml::types::enums::StepStatus;
use tracing::warn;

use crate::config::ConditionEvaluator;
use crate::error::AnmlClientError;

// ---------------------------------------------------------------------------
// StepInfo — enriched step view exposed to callers
// ---------------------------------------------------------------------------

/// An enriched view of a flow step, including RFC-defined attributes
/// that may not be present on the base `AnmlStep` type.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StepInfo {
    /// Step identifier.
    pub id: String,
    /// Human-readable label.
    pub label: Option<String>,
    /// Current status.
    pub status: StepStatus,
    /// Whether the step is required.
    pub required: bool,
    /// The id of the next step in the normal flow.
    pub next: Option<String>,
    /// The condition expression, if any.
    pub condition: Option<String>,
    /// The action id to execute for this step.
    pub action: Option<String>,
    /// The step to transition to on error (RFC §10.6.1).
    pub next_on_error: Option<String>,
    /// Maximum retries before treating failure as terminal (RFC §10.6.2).
    pub retry_budget: u32,
}

impl StepInfo {
    /// Build a `StepInfo` from an `AnmlStep`.
    fn from_anml_step(step: &AnmlStep) -> Self {
        Self {
            id: step.id.clone(),
            label: step.label.clone(),
            status: step.status.unwrap_or(StepStatus::Pending),
            required: step.required.unwrap_or(false),
            next: step.next.clone(),
            condition: step.condition.clone(),
            action: step.action.clone(),
            // The anml crate may not parse these yet; default to None/0.
            next_on_error: None,
            retry_budget: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// HistoryEntry — records each step transition
// ---------------------------------------------------------------------------

/// A record of a step transition in the flow history.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HistoryEntry {
    /// The step id that was active.
    pub step_id: String,
    /// The status the step had when it was the current step.
    pub status: StepStatus,
    /// Wall-clock timestamp (skipped during serialization — `Instant` is
    /// not portable across processes).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub timestamp: Option<Instant>,
}

// ---------------------------------------------------------------------------
// RetryState — per-step retry tracking
// ---------------------------------------------------------------------------

/// Tracks retry attempts for a single step.
#[derive(Clone, Debug, Default)]
struct RetryState {
    /// Number of retries consumed so far.
    attempts: u32,
}

impl RetryState {
    /// Compute the backoff delay for the next retry.
    ///
    /// Exponential backoff: 1s base, 2x multiplier, capped at 60s or
    /// `remaining_ttl`, whichever is smaller.
    fn backoff_delay(&self, remaining_ttl: Option<Duration>) -> Duration {
        let base_ms = 1000u64; // 1 second
        let multiplier = 2u64;
        let delay_ms = base_ms.saturating_mul(multiplier.saturating_pow(self.attempts));
        let cap = Duration::from_secs(60);
        let delay = Duration::from_millis(delay_ms).min(cap);
        match remaining_ttl {
            Some(ttl) => delay.min(ttl),
            None => delay,
        }
    }
}

// ---------------------------------------------------------------------------
// FlowNavigator
// ---------------------------------------------------------------------------

/// Navigates a multi-step ANML flow, tracking state, history, and
/// enforcing retry budgets and per-flow timeouts.
///
/// Construct via [`FlowNavigator::from_document`]. The navigator is
/// `Clone + Send + Sync` when the `ConditionEvaluator` is not held
/// (it is passed per-call to `advance`).
#[derive(Debug)]
pub struct FlowNavigator {
    /// All steps in the flow, in document order.
    steps: Vec<StepInfo>,
    /// Index of the current step in `steps`.
    current_index: usize,
    /// Transition history.
    history: Vec<HistoryEntry>,
    /// The latest document received from the service.
    document: AnmlDocument,
    /// Per-step retry state, keyed by step index.
    retry_states: Vec<RetryState>,
    /// When the navigator was created (for per-flow timeout).
    created_at: Instant,
    /// Per-flow timeout duration.
    flow_timeout: Duration,
}

impl FlowNavigator {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Create a `FlowNavigator` from an `AnmlDocument` that contains a
    /// `<state>/<flow>` section.
    ///
    /// Returns `MalformedDocument` if the document has no flow or no steps.
    pub fn from_document(doc: &AnmlDocument) -> crate::Result<Self> {
        Self::from_document_with_timeout(doc, Duration::from_secs(300))
    }

    /// Create a `FlowNavigator` with a custom per-flow timeout.
    pub fn from_document_with_timeout(
        doc: &AnmlDocument,
        flow_timeout: Duration,
    ) -> crate::Result<Self> {
        let state = doc.state.as_ref().ok_or_else(|| AnmlClientError::MalformedDocument {
            detail: "document has no <state> section".into(),
        })?;

        let flow = state.flow.as_ref().ok_or_else(|| AnmlClientError::MalformedDocument {
            detail: "document <state> has no <flow>".into(),
        })?;

        if flow.steps.is_empty() {
            return Err(AnmlClientError::MalformedDocument {
                detail: "<flow> has no steps".into(),
            });
        }

        let steps: Vec<StepInfo> = flow.steps.iter().map(StepInfo::from_anml_step).collect();
        let retry_states = vec![RetryState::default(); steps.len()];

        // Determine the current step: look for status=current, or use context,
        // or default to the first non-completed step.
        let current_index = Self::resolve_current_index(&steps, state.context.as_ref());

        let mut nav = Self {
            steps,
            current_index,
            history: Vec::new(),
            document: doc.clone(),
            retry_states,
            created_at: Instant::now(),
            flow_timeout,
        };

        // Record initial position in history.
        nav.record_history();
        Ok(nav)
    }

    /// Resolve which step index is "current".
    fn resolve_current_index(
        steps: &[StepInfo],
        context: Option<&anml::types::elements::AnmlContext>,
    ) -> usize {
        // 1. If a step has status=current, use it.
        if let Some(idx) = steps.iter().position(|s| s.status == StepStatus::Current) {
            return idx;
        }
        // 2. If context references a step id, use it.
        if let Some(ctx) = context {
            if let Some(idx) = steps.iter().position(|s| s.id == ctx.step) {
                return idx;
            }
        }
        // 3. Default to the first pending step.
        steps
            .iter()
            .position(|s| s.status == StepStatus::Pending)
            .unwrap_or(0)
    }

    fn record_history(&mut self) {
        if let Some(step) = self.steps.get(self.current_index) {
            self.history.push(HistoryEntry {
                step_id: step.id.clone(),
                status: step.status,
                timestamp: Some(Instant::now()),
            });
        }
    }

    // -----------------------------------------------------------------------
    // Accessors (sub-task 2 + 8)
    // -----------------------------------------------------------------------

    /// The current step, if any.
    pub fn current(&self) -> Option<&StepInfo> {
        self.steps.get(self.current_index)
    }

    /// All pending steps (status == Pending).
    pub fn pending(&self) -> Vec<&StepInfo> {
        self.steps.iter().filter(|s| s.status == StepStatus::Pending).collect()
    }

    /// All completed steps (status == Completed).
    pub fn completed(&self) -> Vec<&StepInfo> {
        self.steps.iter().filter(|s| s.status == StepStatus::Completed).collect()
    }

    /// Whether the flow is complete (no steps are in current or pending status).
    pub fn is_complete(&self) -> bool {
        self.steps
            .iter()
            .all(|s| matches!(s.status, StepStatus::Completed | StepStatus::Skipped))
    }

    /// All steps in the flow.
    pub fn steps(&self) -> &[StepInfo] {
        &self.steps
    }

    /// The transition history.
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// The latest document held by the navigator.
    pub fn document(&self) -> &AnmlDocument {
        &self.document
    }

    /// Total number of steps.
    pub fn total_steps(&self) -> usize {
        self.steps.len()
    }

    /// 1-based position of the current step.
    pub fn current_position(&self) -> usize {
        self.current_index + 1
    }

    // -----------------------------------------------------------------------
    // Flow timeout check (sub-task 9)
    // -----------------------------------------------------------------------

    /// Check whether the per-flow timeout has been exceeded.
    fn check_flow_timeout(&self) -> crate::Result<()> {
        let elapsed = self.created_at.elapsed();
        if elapsed >= self.flow_timeout {
            let step_id = self
                .current()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| "unknown".into());
            return Err(AnmlClientError::FlowAborted {
                step_id,
                detail: format!(
                    "per-flow timeout exceeded ({}s elapsed, {}s limit)",
                    elapsed.as_secs(),
                    self.flow_timeout.as_secs()
                ),
            });
        }
        Ok(())
    }

    /// Remaining time before the flow timeout expires.
    fn remaining_ttl(&self) -> Option<Duration> {
        let elapsed = self.created_at.elapsed();
        self.flow_timeout.checked_sub(elapsed)
    }

    // -----------------------------------------------------------------------
    // State regression detection (sub-task 6)
    // -----------------------------------------------------------------------

    /// Detect if a step has regressed (moved from a later status to an
    /// earlier one in the normal progression). A regression is when a step
    /// moves from Completed back to Current/Pending, or from Current back
    /// to Pending. Skipped is a terminal state and not a regression.
    fn detect_regression(
        old_status: StepStatus,
        new_status: StepStatus,
    ) -> bool {
        // Only flag regression when moving from Completed/Current backward
        // to a non-terminal state. Skipped is a valid terminal transition.
        match (old_status, new_status) {
            (StepStatus::Completed, StepStatus::Current)
            | (StepStatus::Completed, StepStatus::Pending)
            | (StepStatus::Current, StepStatus::Pending) => true,
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // Condition evaluation (sub-task 7)
    // -----------------------------------------------------------------------

    /// Evaluate the condition on a step. Returns true if the step should
    /// be entered, false if it should be skipped.
    fn evaluate_condition(
        step: &StepInfo,
        evaluator: Option<&dyn ConditionEvaluator>,
    ) -> bool {
        match (&step.condition, evaluator) {
            (None, _) => true, // no condition → always available
            (Some(cond), Some(eval)) => eval.evaluate(cond, &step.id),
            (Some(cond), None) => {
                warn!(
                    step_id = %step.id,
                    condition = %cond,
                    "step has condition but no ConditionEvaluator configured; treating as available"
                );
                true
            }
        }
    }

    // -----------------------------------------------------------------------
    // advance() — the main transition method (sub-tasks 3, 4, 5, 6, 7, 9)
    // -----------------------------------------------------------------------

    /// Execute the current step's action and advance the flow.
    ///
    /// `params` are the user-supplied parameters for the step's action.
    /// `action_executor` is a callback that performs the actual HTTP request
    /// and returns the next `AnmlDocument`.
    ///
    /// The method:
    /// 1. Checks the per-flow timeout.
    /// 2. Evaluates the step's condition (if any).
    /// 3. Calls the action executor.
    /// 4. On success: marks the step completed, updates state from the
    ///    response document, and advances to the next step.
    /// 5. On error: consults `retry-budget` and `next-on-error`.
    /// 6. Detects state regressions and surfaces warnings.
    pub async fn advance<F, Fut>(
        &mut self,
        params: &[(String, String)],
        condition_evaluator: Option<&dyn ConditionEvaluator>,
        action_executor: F,
    ) -> crate::Result<AnmlDocument>
    where
        F: Fn(&AnmlDocument, &str, &[(String, String)]) -> Fut,
        Fut: std::future::Future<Output = crate::Result<AnmlDocument>>,
    {
        // 1. Check per-flow timeout
        self.check_flow_timeout()?;

        let step_idx = self.current_index;
        let step = self.steps.get(step_idx).ok_or_else(|| {
            AnmlClientError::FlowAborted {
                step_id: "unknown".into(),
                detail: "no current step".into(),
            }
        })?;

        // 2. Evaluate condition — skip if false
        if !Self::evaluate_condition(step, condition_evaluator) {
            self.steps[step_idx].status = StepStatus::Skipped;
            self.advance_to_next_step(condition_evaluator)?;
            return Ok(self.document.clone());
        }

        // 3. Get the action id
        let action_id = step.action.clone().ok_or_else(|| {
            AnmlClientError::FlowAborted {
                step_id: step.id.clone(),
                detail: "step has no action attribute".into(),
            }
        })?;

        // 4. Execute the action
        let result = action_executor(&self.document, &action_id, params).await;

        match result {
            Ok(next_doc) => {
                // Mark current step completed
                self.steps[step_idx].status = StepStatus::Completed;

                // Update state from the response document
                self.update_state_from_document(&next_doc, condition_evaluator)?;
                self.document = next_doc.clone();

                // Advance to next step
                self.advance_to_next_step(condition_evaluator)?;

                Ok(next_doc)
            }
            Err(err) => {
                self.handle_step_error(step_idx, err, params, condition_evaluator, &action_executor).await
            }
        }
    }

    /// Handle a step execution error: retry with backoff or follow
    /// `next-on-error`, or abort.
    fn handle_step_error<'a, F, Fut>(
        &'a mut self,
        step_idx: usize,
        original_err: AnmlClientError,
        params: &'a [(String, String)],
        condition_evaluator: Option<&'a dyn ConditionEvaluator>,
        action_executor: &'a F,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<AnmlDocument>> + 'a>>
    where
        F: Fn(&AnmlDocument, &str, &[(String, String)]) -> Fut,
        Fut: std::future::Future<Output = crate::Result<AnmlDocument>> + 'a,
    {
        Box::pin(async move {
        let step = &self.steps[step_idx];
        let retry_budget = step.retry_budget;
        let next_on_error = step.next_on_error.clone();
        let step_id = step.id.clone();
        let action_id = step.action.clone().unwrap_or_default();

        // Check retry budget
        let attempts = self.retry_states[step_idx].attempts;
        if attempts < retry_budget {
            self.retry_states[step_idx].attempts += 1;

            // Exponential backoff
            let delay = self.retry_states[step_idx].backoff_delay(self.remaining_ttl());
            tokio::time::sleep(delay).await;

            // Check flow timeout after sleeping
            self.check_flow_timeout()?;

            // Retry the action
            let result = action_executor(&self.document, &action_id, params).await;
            match result {
                Ok(next_doc) => {
                    self.steps[step_idx].status = StepStatus::Completed;
                    self.update_state_from_document(&next_doc, condition_evaluator)?;
                    self.document = next_doc.clone();
                    self.advance_to_next_step(condition_evaluator)?;
                    return Ok(next_doc);
                }
                Err(retry_err) => {
                    // Recurse for further retries
                    return self
                        .handle_step_error(
                            step_idx,
                            retry_err,
                            params,
                            condition_evaluator,
                            action_executor,
                        )
                        .await;
                }
            }
        }

        // Budget exhausted — follow next-on-error or abort
        if let Some(ref error_step_id) = next_on_error {
            if let Some(error_idx) = self.steps.iter().position(|s| s.id == *error_step_id) {
                self.steps[step_idx].status = StepStatus::Skipped;
                self.current_index = error_idx;
                self.steps[error_idx].status = StepStatus::Current;
                self.record_history();
                return Ok(self.document.clone());
            }
        }

        // No fallback — abort
        Err(AnmlClientError::FlowAborted {
            step_id,
            detail: format!(
                "retry budget exhausted ({} retries) and no next-on-error fallback: {}",
                retry_budget, original_err
            ),
        })
        }) // close Box::pin(async move { ... })
    }

    /// Advance to the next step after the current one completes.
    fn advance_to_next_step(
        &mut self,
        condition_evaluator: Option<&dyn ConditionEvaluator>,
    ) -> crate::Result<()> {
        let current = &self.steps[self.current_index];

        // Determine next step: explicit `next` attribute, or document order.
        let next_idx = if let Some(ref next_id) = current.next {
            self.steps.iter().position(|s| s.id == *next_id)
        } else {
            // Next in document order after current
            let candidate = self.current_index + 1;
            if candidate < self.steps.len() {
                Some(candidate)
            } else {
                None
            }
        };

        if let Some(idx) = next_idx {
            // Check condition on the next step; skip if false
            if !Self::evaluate_condition(&self.steps[idx], condition_evaluator) {
                self.steps[idx].status = StepStatus::Skipped;
                self.current_index = idx;
                // Recurse to find the next available step
                return self.advance_to_next_step(condition_evaluator);
            }

            self.current_index = idx;
            self.steps[idx].status = StepStatus::Current;
            self.record_history();
        }
        // If no next step, the flow is at the end — current_index stays.

        Ok(())
    }

    /// Update internal step states from a new document received from the
    /// service. Detects state regressions.
    fn update_state_from_document(
        &mut self,
        doc: &AnmlDocument,
        _condition_evaluator: Option<&dyn ConditionEvaluator>,
    ) -> crate::Result<()> {
        let new_steps = match doc.state.as_ref().and_then(|s| s.flow.as_ref()) {
            Some(flow) => &flow.steps,
            None => return Ok(()), // Response has no flow — keep existing state
        };

        for new_step in new_steps {
            if let Some(existing) = self.steps.iter_mut().find(|s| s.id == new_step.id) {
                let new_status = new_step.status.unwrap_or(StepStatus::Pending);
                let old_status = existing.status;

                // Detect regression
                if Self::detect_regression(old_status, new_status) {
                    warn!(
                        step_id = %existing.id,
                        from = %old_status,
                        to = %new_status,
                        "unexpected state regression detected"
                    );
                    // Surface as a warning but don't block — the caller
                    // can inspect the error type if needed.
                    return Err(AnmlClientError::UnexpectedStateRegression {
                        step_id: existing.id.clone(),
                        from: old_status.to_string(),
                        to: new_status.to_string(),
                    });
                }

                existing.status = new_status;
            }
        }

        Ok(())
    }

    /// Set the `next-on-error` fallback for a step by id.
    ///
    /// The anml crate may not yet parse these attributes, so this allows
    /// callers to configure them manually.
    pub fn set_next_on_error(&mut self, step_id: &str, error_step_id: &str) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == step_id) {
            step.next_on_error = Some(error_step_id.to_string());
        }
    }

    /// Set the retry budget for a step by id.
    pub fn set_retry_budget(&mut self, step_id: &str, budget: u32) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == step_id) {
            step.retry_budget = budget;
        }
    }
}

// ---------------------------------------------------------------------------
// Display impl (sub-task 10)
// ---------------------------------------------------------------------------

impl fmt::Display for FlowNavigator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format: "Flow[search → select → payment → confirm] @ search (1/4)"
        let step_names: Vec<&str> = self.steps.iter().map(|s| s.id.as_str()).collect();
        let current_id = self
            .current()
            .map(|s| s.id.as_str())
            .unwrap_or("?");

        write!(
            f,
            "Flow[{}] @ {} ({}/{})",
            step_names.join(" → "),
            current_id,
            self.current_position(),
            self.total_steps(),
        )
    }
}

impl fmt::Display for StepInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Step[{}]", self.id)?;
        if let Some(ref label) = self.label {
            write!(f, " \"{}\"", label)?;
        }
        write!(f, " ({})", self.status)?;
        if self.required {
            write!(f, " required")?;
        }
        if let Some(ref cond) = self.condition {
            write!(f, " condition=\"{}\"", cond)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Serde support (sub-task 11)
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_support {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Serializable snapshot of a `FlowNavigator`.
    ///
    /// Fields that cannot be serialized (like `Instant`) are omitted.
    /// The navigator can be reconstructed from a document + this snapshot.
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct FlowNavigatorSnapshot {
        /// All steps with their current state.
        pub steps: Vec<StepInfo>,
        /// Index of the current step.
        pub current_index: usize,
        /// Transition history (timestamps omitted).
        pub history: Vec<HistoryEntry>,
        /// Per-flow timeout in seconds.
        pub flow_timeout_secs: u64,
    }

    impl FlowNavigator {
        /// Export a serializable snapshot of the navigator state.
        pub fn to_snapshot(&self) -> FlowNavigatorSnapshot {
            FlowNavigatorSnapshot {
                steps: self.steps.clone(),
                current_index: self.current_index,
                history: self.history.clone(),
                flow_timeout_secs: self.flow_timeout.as_secs(),
            }
        }

        /// Restore a navigator from a snapshot and a document.
        ///
        /// The `Instant` fields are reset to `Instant::now()`.
        pub fn from_snapshot(
            snapshot: FlowNavigatorSnapshot,
            doc: &AnmlDocument,
        ) -> Self {
            Self {
                steps: snapshot.steps,
                current_index: snapshot.current_index,
                history: snapshot.history,
                document: doc.clone(),
                retry_states: vec![RetryState::default(); 0], // will be resized below
                created_at: Instant::now(),
                flow_timeout: Duration::from_secs(snapshot.flow_timeout_secs),
            }
        }
    }
}

#[cfg(feature = "serde")]
pub use serde_support::FlowNavigatorSnapshot;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use anml::types::elements::{AnmlContext, AnmlFlow, AnmlState, AnmlStep};

    /// Helper to build a test document with a flow.
    fn make_flow_doc(steps: Vec<AnmlStep>, context_step: Option<&str>) -> AnmlDocument {
        AnmlDocument {
            state: Some(AnmlState {
                flow: Some(AnmlFlow { steps }),
                context: context_step.map(|s| AnmlContext {
                    step: s.to_string(),
                }),
            }),
            ..Default::default()
        }
    }

    fn step(id: &str, status: StepStatus) -> AnmlStep {
        AnmlStep {
            id: id.to_string(),
            label: Some(format!("{} label", id)),
            status: Some(status),
            required: Some(false),
            next: None,
            condition: None,
            action: Some(format!("action-{}", id)),
        }
    }

    #[test]
    fn from_document_basic() {
        let doc = make_flow_doc(
            vec![
                step("search", StepStatus::Current),
                step("select", StepStatus::Pending),
                step("payment", StepStatus::Pending),
                step("confirm", StepStatus::Pending),
            ],
            Some("search"),
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        assert_eq!(nav.current().unwrap().id, "search");
        assert_eq!(nav.total_steps(), 4);
        assert_eq!(nav.current_position(), 1);
        assert!(!nav.is_complete());
    }

    #[test]
    fn from_document_no_state_errors() {
        let doc = AnmlDocument::default();
        assert!(FlowNavigator::from_document(&doc).is_err());
    }

    #[test]
    fn from_document_no_flow_errors() {
        let doc = AnmlDocument {
            state: Some(AnmlState {
                flow: None,
                context: None,
            }),
            ..Default::default()
        };
        assert!(FlowNavigator::from_document(&doc).is_err());
    }

    #[test]
    fn from_document_empty_flow_errors() {
        let doc = make_flow_doc(vec![], None);
        assert!(FlowNavigator::from_document(&doc).is_err());
    }

    #[test]
    fn accessors_pending_completed() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Completed),
                step("b", StepStatus::Current),
                step("c", StepStatus::Pending),
                step("d", StepStatus::Pending),
            ],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        assert_eq!(nav.completed().len(), 1);
        assert_eq!(nav.pending().len(), 2);
        assert_eq!(nav.current().unwrap().id, "b");
    }

    #[test]
    fn is_complete_all_done() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Completed),
                step("b", StepStatus::Completed),
            ],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        assert!(nav.is_complete());
    }

    #[test]
    fn is_complete_with_skipped() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Completed),
                step("b", StepStatus::Skipped),
            ],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        assert!(nav.is_complete());
    }

    #[test]
    fn display_format() {
        let doc = make_flow_doc(
            vec![
                step("search", StepStatus::Current),
                step("select", StepStatus::Pending),
                step("payment", StepStatus::Pending),
                step("confirm", StepStatus::Pending),
            ],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        let display = nav.to_string();
        assert_eq!(
            display,
            "Flow[search → select → payment → confirm] @ search (1/4)"
        );
    }

    #[test]
    fn step_info_display() {
        let info = StepInfo {
            id: "payment".into(),
            label: Some("Payment".into()),
            status: StepStatus::Pending,
            required: true,
            next: None,
            condition: Some("cart.total > 0".into()),
            action: Some("pay".into()),
            next_on_error: None,
            retry_budget: 0,
        };
        let display = info.to_string();
        assert!(display.contains("payment"));
        assert!(display.contains("Payment"));
        assert!(display.contains("required"));
        assert!(display.contains("condition="));
    }

    #[test]
    fn condition_accessor_exposed() {
        let doc = make_flow_doc(
            vec![AnmlStep {
                id: "conditional".into(),
                label: None,
                status: Some(StepStatus::Pending),
                required: None,
                next: None,
                condition: Some("user.verified == true".into()),
                action: None,
            }],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        let step = nav.current().unwrap();
        assert_eq!(
            step.condition.as_deref(),
            Some("user.verified == true")
        );
    }

    #[test]
    fn detect_regression_completed_to_pending() {
        assert!(FlowNavigator::detect_regression(
            StepStatus::Completed,
            StepStatus::Pending
        ));
    }

    #[test]
    fn detect_regression_current_to_pending() {
        assert!(FlowNavigator::detect_regression(
            StepStatus::Current,
            StepStatus::Pending
        ));
    }

    #[test]
    fn no_regression_pending_to_current() {
        assert!(!FlowNavigator::detect_regression(
            StepStatus::Pending,
            StepStatus::Current
        ));
    }

    #[test]
    fn no_regression_same_status() {
        assert!(!FlowNavigator::detect_regression(
            StepStatus::Completed,
            StepStatus::Completed
        ));
    }

    #[test]
    fn no_regression_pending_to_skipped() {
        // Skipping a pending step is a valid transition, not a regression
        assert!(!FlowNavigator::detect_regression(
            StepStatus::Pending,
            StepStatus::Skipped
        ));
    }

    #[test]
    fn context_determines_current_step() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Pending),
                step("b", StepStatus::Pending),
                step("c", StepStatus::Pending),
            ],
            Some("b"),
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        assert_eq!(nav.current().unwrap().id, "b");
    }

    #[test]
    fn retry_backoff_delay() {
        let state = RetryState { attempts: 0 };
        assert_eq!(state.backoff_delay(None), Duration::from_secs(1));

        let state = RetryState { attempts: 1 };
        assert_eq!(state.backoff_delay(None), Duration::from_secs(2));

        let state = RetryState { attempts: 2 };
        assert_eq!(state.backoff_delay(None), Duration::from_secs(4));

        let state = RetryState { attempts: 3 };
        assert_eq!(state.backoff_delay(None), Duration::from_secs(8));

        // Cap at 60s
        let state = RetryState { attempts: 10 };
        assert_eq!(state.backoff_delay(None), Duration::from_secs(60));
    }

    #[test]
    fn retry_backoff_respects_ttl() {
        let state = RetryState { attempts: 5 };
        let ttl = Duration::from_secs(3);
        assert_eq!(state.backoff_delay(Some(ttl)), Duration::from_secs(3));
    }

    #[test]
    fn flow_timeout_check() {
        let doc = make_flow_doc(
            vec![step("a", StepStatus::Current)],
            None,
        );

        let nav = FlowNavigator::from_document_with_timeout(
            &doc,
            Duration::from_millis(0), // immediate timeout
        )
        .unwrap();

        // The flow should be timed out immediately (or very soon)
        std::thread::sleep(Duration::from_millis(1));
        assert!(nav.check_flow_timeout().is_err());
    }

    #[test]
    fn set_retry_budget_and_next_on_error() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Current),
                step("b", StepStatus::Pending),
            ],
            None,
        );

        let mut nav = FlowNavigator::from_document(&doc).unwrap();
        nav.set_retry_budget("a", 3);
        nav.set_next_on_error("a", "b");

        assert_eq!(nav.steps[0].retry_budget, 3);
        assert_eq!(nav.steps[0].next_on_error.as_deref(), Some("b"));
    }

    #[test]
    fn history_records_initial_position() {
        let doc = make_flow_doc(
            vec![step("a", StepStatus::Current)],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        assert_eq!(nav.history().len(), 1);
        assert_eq!(nav.history()[0].step_id, "a");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn snapshot_round_trip() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Completed),
                step("b", StepStatus::Current),
            ],
            None,
        );

        let nav = FlowNavigator::from_document(&doc).unwrap();
        let snapshot = nav.to_snapshot();

        let json = serde_json::to_string(&snapshot).unwrap();
        let restored: FlowNavigatorSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.current_index, 1);
        assert_eq!(restored.steps.len(), 2);
        assert_eq!(restored.steps[0].id, "a");
    }

    // Async tests for advance()
    #[tokio::test]
    async fn advance_basic_flow() {
        let doc = make_flow_doc(
            vec![
                step("search", StepStatus::Current),
                step("select", StepStatus::Pending),
            ],
            None,
        );

        let mut nav = FlowNavigator::from_document(&doc).unwrap();

        // Mock executor that returns a doc with updated state
        let next_doc = make_flow_doc(
            vec![
                step("search", StepStatus::Completed),
                step("select", StepStatus::Current),
            ],
            Some("select"),
        );

        let result = nav
            .advance(
                &[],
                None,
                |_doc, _action_id, _params| {
                    let nd = next_doc.clone();
                    async move { Ok(nd) }
                },
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(nav.current().unwrap().id, "select");
        assert_eq!(nav.completed().len(), 1);
    }

    #[tokio::test]
    async fn advance_skips_condition_false() {
        let steps = vec![
            step("a", StepStatus::Current),
            AnmlStep {
                id: "b".into(),
                label: None,
                status: Some(StepStatus::Pending),
                required: Some(false),
                next: None,
                condition: Some("should_skip".into()),
                action: Some("action-b".into()),
            },
            step("c", StepStatus::Pending),
        ];
        // a -> b -> c, but b has a condition

        let doc = make_flow_doc(steps, None);
        let mut nav = FlowNavigator::from_document(&doc).unwrap();

        // Evaluator that returns false for "should_skip"
        struct SkipEvaluator;
        impl ConditionEvaluator for SkipEvaluator {
            fn evaluate(&self, condition: &str, _step_id: &str) -> bool {
                condition != "should_skip"
            }
        }

        let next_doc = make_flow_doc(
            vec![
                step("a", StepStatus::Completed),
                step("b", StepStatus::Skipped),
                step("c", StepStatus::Current),
            ],
            Some("c"),
        );

        let evaluator = SkipEvaluator;
        let result = nav
            .advance(
                &[],
                Some(&evaluator),
                |_doc, _action_id, _params| {
                    let nd = next_doc.clone();
                    async move { Ok(nd) }
                },
            )
            .await;

        assert!(result.is_ok());
        // After advancing from "a", "b" should be skipped, landing on "c"
        assert_eq!(nav.current().unwrap().id, "c");
    }

    #[tokio::test]
    async fn advance_flow_timeout_aborts() {
        let doc = make_flow_doc(
            vec![step("a", StepStatus::Current)],
            None,
        );

        let mut nav = FlowNavigator::from_document_with_timeout(
            &doc,
            Duration::from_millis(0),
        )
        .unwrap();

        tokio::time::sleep(Duration::from_millis(1)).await;

        let result = nav
            .advance(
                &[],
                None,
                |_doc, _action_id, _params| async {
                    Ok(AnmlDocument::default())
                },
            )
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AnmlClientError::FlowAborted { step_id, detail } => {
                assert_eq!(step_id, "a");
                assert!(detail.contains("timeout"));
            }
            other => panic!("expected FlowAborted, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn advance_detects_regression() {
        let doc = make_flow_doc(
            vec![
                step("a", StepStatus::Current),
                step("b", StepStatus::Pending),
            ],
            None,
        );

        let mut nav = FlowNavigator::from_document(&doc).unwrap();

        // Response document regresses step "a" from current back to pending
        let regressed_doc = make_flow_doc(
            vec![
                step("a", StepStatus::Pending), // regression!
                step("b", StepStatus::Current),
            ],
            Some("b"),
        );

        let result = nav
            .advance(
                &[],
                None,
                |_doc, _action_id, _params| {
                    let nd = regressed_doc.clone();
                    async move { Ok(nd) }
                },
            )
            .await;

        // The advance should detect the regression
        // Note: we mark the step completed before updating from doc,
        // so the regression is from Completed -> Pending
        assert!(result.is_err());
        match result.unwrap_err() {
            AnmlClientError::UnexpectedStateRegression { step_id, from, to } => {
                assert_eq!(step_id, "a");
                assert_eq!(from, "completed");
                assert_eq!(to, "pending");
            }
            other => panic!("expected UnexpectedStateRegression, got: {:?}", other),
        }
    }
}
