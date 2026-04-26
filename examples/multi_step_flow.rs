//! Multi-step flow navigation with error recovery.
//!
//! ```no_run
//! cargo run --example multi_step_flow
//! ```

use anml_client::flow::FlowNavigator;

#[tokio::main]
async fn main() -> anml_client::Result<()> {
    // Simulate a multi-step flow document
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<anml version="1.0">
  <head><title>Booking Flow</title></head>
  <interact>
    <action id="do-search" method="POST" endpoint="/search">
      <param name="query" type="string" required="true"/>
    </action>
    <action id="do-select" method="POST" endpoint="/select">
      <param name="item_id" type="string" required="true"/>
    </action>
    <action id="do-confirm" method="POST" endpoint="/confirm"/>
  </interact>
  <state>
    <flow>
      <step id="search" status="current" action="do-search" label="Search"/>
      <step id="select" status="pending" action="do-select" label="Select"/>
      <step id="confirm" status="pending" action="do-confirm" label="Confirm"/>
    </flow>
    <context step="search"/>
  </state>
</anml>"#;

    let doc = anml::parser::parse(xml).map_err(anml_client::AnmlClientError::from)?;

    // Create a flow navigator
    let nav = FlowNavigator::from_document(&doc)?;

    println!("Flow: {nav}");
    println!("Current step: {:?}", nav.current().map(|s| &s.id));
    println!("Total steps: {}", nav.total_steps());
    println!("Is complete: {}", nav.is_complete());

    // List all steps
    for step in nav.steps() {
        println!(
            "  Step '{}' ({}) - action: {:?}",
            step.id, step.status, step.action
        );
    }

    // Pending steps
    let pending = nav.pending();
    println!("Pending steps: {}", pending.len());
    for step in &pending {
        println!("  - {}", step.id);
    }

    // In a real scenario, you'd call nav.advance() with an action executor:
    // let next_doc = nav.advance(&params, None, |doc, action_id, params| async {
    //     client.execute_action(doc, action_id, params, &ctx).await
    // }).await?;

    Ok(())
}
