//! Pre-built ANML document fixtures for testing.
//!
//! Each fixture returns a valid ANML XML string suitable for use with
//! [`MockAnmlServer`](super::MockAnmlServer).

/// The ANML namespace URI.
const NS: &str = "urn:ietf:params:xml:ns:anml:1.0";

/// A simple service document with one ask and one action.
pub fn simple_service() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head>
    <title>Simple Test Service</title>
    <meta name="profile" value="core-1.0"/>
  </head>
  <knowledge>
    <ask field="airline" action="submit-airline" required="true">
      Which airline do you prefer?
    </ask>
  </knowledge>
  <interact>
    <action id="submit-airline" method="POST" endpoint="/airline">
      <param name="airline" type="string" required="true"/>
    </action>
  </interact>
</anml>"#
    )
}

/// A multi-step flow document with 3 steps.
pub fn multi_step_flow() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head>
    <title>Multi-Step Flow</title>
    <meta name="profile" value="core-1.0"/>
  </head>
  <knowledge>
    <ask field="search_query" action="do-search" required="true">
      What are you looking for?
    </ask>
  </knowledge>
  <interact>
    <action id="do-search" method="POST" endpoint="/search">
      <param name="search_query" type="string" required="true"/>
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
</anml>"#
    )
}

/// A document with disclosure constraints requiring explicit consent.
pub fn disclosure_gated() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head>
    <title>Disclosure-Gated Service</title>
    <meta name="profile" value="core-1.0"/>
  </head>
  <constraints>
    <disclosure field="email" requires="explicit"/>
    <disclosure field="name" requires="implicit"/>
  </constraints>
  <knowledge>
    <ask field="email" action="submit-info" required="true">
      Your email address
    </ask>
    <ask field="name" action="submit-info">
      Your name
    </ask>
  </knowledge>
  <interact>
    <action id="submit-info" method="POST" endpoint="/info">
      <param name="email" type="string" required="true"/>
      <param name="name" type="string"/>
    </action>
  </interact>
</anml>"#
    )
}

/// A paginated document (page 1 of 3).
pub fn paginated_page1() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head><title>Paginated Results</title></head>
  <body>
    <data id="results">
      <inform>Item 1</inform>
      <inform>Item 2</inform>
    </data>
    <nav next="/results?page=2" total="6"/>
  </body>
</anml>"#
    )
}

/// Paginated document page 2.
pub fn paginated_page2() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head><title>Paginated Results</title></head>
  <body>
    <data id="results">
      <inform>Item 3</inform>
      <inform>Item 4</inform>
    </data>
    <nav next="/results?page=3" total="6"/>
  </body>
</anml>"#
    )
}

/// Paginated document page 3 (last page, no next).
pub fn paginated_page3() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head><title>Paginated Results</title></head>
  <body>
    <data id="results">
      <inform>Item 5</inform>
      <inform>Item 6</inform>
    </data>
    <nav total="6"/>
  </body>
</anml>"#
    )
}

/// An error/problem response document.
pub fn error_problem() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head><title>Error</title></head>
  <status code="406">
    <message>Unsupported version</message>
  </status>
</anml>"#
    )
}

/// A document requiring an unsupported extension.
pub fn extension_required() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head>
    <title>Extension Required</title>
    <meta name="requires-ext" value="https://example.com/anml/ext/payments/1"/>
  </head>
</anml>"#
    )
}

/// A deferred ask document (ask without action attribute).
pub fn deferred_ask() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head>
    <title>Deferred Ask</title>
    <meta name="profile" value="core-1.0"/>
  </head>
  <knowledge>
    <ask field="preference" required="false">
      What is your preference?
    </ask>
  </knowledge>
</anml>"#
    )
}

/// A success status response (for action responses).
pub fn success_response() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head><title>Success</title></head>
  <status code="200">
    <message>Action completed successfully</message>
  </status>
</anml>"#
    )
}

/// A document requiring an unsupported profile.
pub fn unsupported_profile() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="{NS}" version="1.0">
  <head>
    <title>Unsupported Profile</title>
    <meta name="profile" value="urn:ietf:anml:profile:signed-answer-1.0"/>
  </head>
</anml>"#
    )
}
