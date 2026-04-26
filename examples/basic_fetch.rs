//! Basic ANML client usage: discover, fetch, and inspect a document.
//!
//! ```no_run
//! cargo run --example basic_fetch
//! ```

use anml_client::prelude::*;

#[tokio::main]
async fn main() -> anml_client::Result<()> {
    // Build a client with an allow-list trust policy
    let client = AnmlClient::builder()
        .base_url("https://api.example.com")
        .trust_policy(
            AllowListTrustPolicy::new().allow_url("https://api.example.com"),
        )
        .build()?;

    // Discover the ANML service
    match client.discover("https://api.example.com").await {
        Ok(discovery) => {
            println!("Discovered ANML endpoint: {}", discovery.endpoint);
            println!("Version: {:?}", discovery.version);
        }
        Err(e) => {
            println!("Discovery failed (expected without a real server): {e}");
        }
    }

    // Fetch an ANML document
    match client.fetch("/service").await {
        Ok(doc) => {
            // Inspect the document
            let title = doc
                .head
                .as_ref()
                .and_then(|h| h.title.as_ref())
                .map(|t| t.text.as_str())
                .unwrap_or("(untitled)");
            println!("Document title: {title}");

            // List asks
            if let Some(ref knowledge) = doc.knowledge {
                if let Some(ref asks) = knowledge.asks {
                    println!("Service asks for {} fields:", asks.len());
                    for ask in asks {
                        println!("  - {} (action: {})", ask.field, ask.action);
                    }
                }
            }

            // List actions
            if let Some(ref interact) = doc.interact {
                println!("Available actions:");
                for action in &interact.actions {
                    println!(
                        "  - {} {} {}",
                        action.id, action.method, action.endpoint
                    );
                }
            }
        }
        Err(e) => {
            println!("Fetch failed (expected without a real server): {e}");
        }
    }

    Ok(())
}
