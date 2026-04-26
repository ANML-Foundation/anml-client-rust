//! Action execution with disclosure and response handling.
//!
//! ```no_run
//! cargo run --example action_execution
//! ```

use anml_client::prelude::*;
use anml_client::config::{ConsentDecision, ConsentHandler, Origin};

/// A consent handler that always grants consent (for demo purposes).
struct AutoGrantConsent;

impl ConsentHandler for AutoGrantConsent {
    fn request_consent(
        &self,
        field: &str,
        origin: &Origin,
        purpose: Option<&str>,
    ) -> ConsentDecision {
        println!(
            "Consent requested for field '{}' to origin '{}' (purpose: {:?})",
            field, origin, purpose
        );
        ConsentDecision::Grant
    }
}

#[tokio::main]
async fn main() -> anml_client::Result<()> {
    let client = AnmlClient::builder()
        .base_url("https://api.example.com")
        .trust_policy(
            AllowListTrustPolicy::new().allow_url("https://api.example.com"),
        )
        .consent_handler(AutoGrantConsent)
        .build()?;

    // Fetch the service document
    match client.fetch("/service").await {
        Ok(doc) => {
            println!("Fetched service document");

            // Build and execute an action using the fluent builder
            let builder = ActionRequestBuilder::new(
                &doc,
                "submit-airline",
                "https://api.example.com",
                None,
            );

            match builder {
                Ok(b) => {
                    let b = b
                        .param("airline", "Delta")
                        .with_consent(anml_client::disclosure::ConsentBasis::Explicit);
                    println!("Action builder configured with params");
                    // In a real scenario: b.execute(&ctx).await?
                    let _ = b;
                }
                Err(e) => {
                    println!("Builder error (expected without matching doc): {e}");
                }
            }
        }
        Err(e) => {
            println!("Fetch failed (expected without a real server): {e}");
        }
    }

    Ok(())
}
