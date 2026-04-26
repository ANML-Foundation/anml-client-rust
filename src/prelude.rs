//! Convenience re-exports for common types.
//!
//! Import everything you need for typical ANML client usage with a single
//! glob import:
//!
//! ```rust
//! use anml_client::prelude::*;
//! ```
//!
//! This re-exports the core client types, configuration traits, error
//! types, and the most commonly used `anml` crate types (document and
//! knowledge primitives).

// -- Core client types --
pub use crate::client::AnmlClient;
pub use crate::client::DocumentSummary;
pub use crate::config::{AllowListTrustPolicy, ClientConfig, TrustPolicy};
pub use crate::error::{AnmlClientError, Result};

// -- anml crate types: document and knowledge primitives --
pub use anml::types::document::AnmlDocument;
pub use anml::types::elements::{AnmlAction, AnmlAnswer, AnmlAsk, AnmlInform, AnmlRefuse};

// -- Disclosure types --
pub use crate::disclosure::{ConsentBasis, DisclosureDecision};

// -- Action builder --
pub use crate::action::builder::ActionRequestBuilder;

// -- Flow navigation --
pub use crate::flow::FlowNavigator;
