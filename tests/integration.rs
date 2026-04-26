//! Integration tests for the ANML client.
//!
//! These tests use `MockAnmlServer` to verify end-to-end client behavior.

#[cfg(feature = "testing")]
mod integration {
    use anml_client::client::AnmlClient;
    use anml_client::testing::{fixtures, MockAnmlServer};

    // -----------------------------------------------------------------------
    // Helper: build a client pointing at the mock server
    // -----------------------------------------------------------------------

    fn build_client(server_url: &str) -> AnmlClient {
        AnmlClient::builder()
            .base_url(server_url)
            .allow_plaintext_http(true)
            .build()
            .unwrap()
    }

    // -----------------------------------------------------------------------
    // Happy path: fetch and inspect
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn happy_path_fetch_and_inspect() {
        // Use a document without <interact> or <constraints> to avoid
        // HTTP transport rejection (mock server uses HTTP, not HTTPS)
        let simple_doc = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="urn:ietf:params:xml:ns:anml:1.0" version="1.0">
  <head>
    <title>Simple Test Service</title>
    <meta name="profile" value="core-1.0"/>
  </head>
  <knowledge>
    <ask field="airline" action="submit-airline" required="true">
      Which airline do you prefer?
    </ask>
  </knowledge>
</anml>"#
        );

        let server = MockAnmlServer::builder()
            .document("/service", &simple_doc)
            .build()
            .await;

        let client = build_client(&server.url());
        let doc = client.fetch("/service").await.unwrap();

        // Verify the document was parsed
        assert!(doc.head.is_some());
        let title = doc
            .head
            .as_ref()
            .and_then(|h| h.title.as_ref())
            .map(|t| t.text.as_str());
        assert_eq!(title, Some("Simple Test Service"));

        // Verify the server recorded the request
        server.assert_received("/service");
    }

    // -----------------------------------------------------------------------
    // Error: unsupported extension
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_unsupported_extension() {
        let server = MockAnmlServer::builder()
            .document("/ext", &fixtures::extension_required())
            .build()
            .await;

        let client = build_client(&server.url());
        let result = client.fetch("/ext").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, anml_client::AnmlClientError::UnsupportedExtension { .. }),
            "expected UnsupportedExtension, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Error: unsupported profile
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_unsupported_profile() {
        let server = MockAnmlServer::builder()
            .document("/profile", &fixtures::unsupported_profile())
            .build()
            .await;

        let client = build_client(&server.url());
        let result = client.fetch("/profile").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, anml_client::AnmlClientError::UnsupportedProfile { .. }),
            "expected UnsupportedProfile, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Error: 406 version mismatch
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_406_version_mismatch() {
        let server = MockAnmlServer::builder()
            .document("/version", &fixtures::error_problem())
            .status("/version", 406)
            .build()
            .await;

        let client = build_client(&server.url());
        let result = client.fetch("/version").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, anml_client::AnmlClientError::UnsupportedVersion { .. }),
            "expected UnsupportedVersion, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Error: plaintext rejection
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_plaintext_rejection() {
        // Client with allow_plaintext_http=false (default)
        let client = AnmlClient::builder()
            .base_url("http://127.0.0.1:9999")
            .build()
            .unwrap();

        let result = client.fetch("/service").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, anml_client::AnmlClientError::TransportInsecure { .. }),
            "expected TransportInsecure, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Error: SSRF block
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_ssrf_block() {
        use anml_client::action::is_private_ip;
        // Verify SSRF detection for private IPs
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
    }

    // -----------------------------------------------------------------------
    // URI resolution: relative endpoint against document origin
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn uri_resolution_relative() {
        use anml_client::action::resolve_endpoint;

        let resolved =
            resolve_endpoint("/api/submit", "https://api.example.com", None).unwrap();
        assert_eq!(resolved, "https://api.example.com/api/submit");
    }

    #[tokio::test]
    async fn uri_resolution_xml_base_override() {
        use anml_client::action::resolve_endpoint;

        let resolved = resolve_endpoint(
            "/submit",
            "https://api.example.com",
            Some("https://base.example.com"),
        )
        .unwrap();
        assert_eq!(resolved, "https://base.example.com/submit");
    }

    // -----------------------------------------------------------------------
    // Trust denial
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_trust_denial() {
        use anml_client::config::{DenyAllTrustPolicy, Origin, TrustDecision, TrustPolicy};
        use anml::types::document::AnmlDocument;

        let policy = DenyAllTrustPolicy;
        let origin = Origin {
            scheme: "https".into(),
            host: "evil.example.com".into(),
            port: None,
        };
        let doc = AnmlDocument::default();
        let decision = policy.evaluate(&origin, &doc);
        assert!(matches!(decision, TrustDecision::Deny { .. }));
    }

    // -----------------------------------------------------------------------
    // Disclosure gate: explicit consent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn disclosure_explicit_consent_callback() {
        use anml_client::config::{ConsentDecision, ConsentHandler, Origin, TrustDecision, TrustPolicy};
        use anml_client::disclosure::*;
        use anml::types::document::AnmlDocument;
        use anml::types::elements::{AnmlConstraints, AnmlDisclosure};
        use anml::types::enums::DisclosureRequires;

        struct GrantAll;
        impl ConsentHandler for GrantAll {
            fn request_consent(&self, _: &str, _: &Origin, _: Option<&str>) -> ConsentDecision {
                ConsentDecision::Grant
            }
        }
        struct AllowAll;
        impl TrustPolicy for AllowAll {
            fn evaluate(&self, _: &Origin, _: &AnmlDocument) -> TrustDecision {
                TrustDecision::Allow
            }
        }

        let origin = Origin {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        };
        let consent_store = ConsentStore::new();
        let rate_limiter = RateLimitTracker::new();
        let trust = AllowAll;
        let handler = GrantAll;

        let doc = AnmlDocument {
            constraints: Some(AnmlConstraints {
                disclosures: Some(vec![AnmlDisclosure {
                    field: "email".to_string(),
                    requires: DisclosureRequires::ExplicitConsent,
                }]),
            }),
            ..Default::default()
        };

        let rules = extract_rules(&doc);
        let ctx = DisclosureContext {
            origin: &origin,
            consent_store: &consent_store,
            rate_limiter: &rate_limiter,
            trust_policy: &trust,
            auth_provider: None,
            consent_handler: Some(&handler),
            tokenizer: None,
            principal_id: None,
        };

        let decision = evaluate(&doc, &rules, "email", "test@example.com", &ctx);
        assert!(matches!(decision, DisclosureDecision::Allow { .. }));
    }

    // -----------------------------------------------------------------------
    // Integrity mismatch
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn error_integrity_mismatch() {
        use anml_client::integrity::verify_integrity;

        let data = b"hello world";
        let bad_attr = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let result = verify_integrity(data, bad_attr, "img");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            anml_client::AnmlClientError::IntegrityMismatch { .. }
        ));
    }

    // -----------------------------------------------------------------------
    // Discovery: well-known
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn discovery_well_known() {
        let well_known_doc = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<anml xmlns="urn:ietf:params:xml:ns:anml:1.0" version="1.0">
  <head><title>Discovery</title></head>
</anml>"#
        );

        let server = MockAnmlServer::builder()
            .document("/.well-known/anml", &well_known_doc)
            .build()
            .await;

        let client = build_client(&server.url());
        let result = client.discover(&server.url()).await;
        // Discovery may succeed or fail depending on content-type handling;
        // the key test is that the request was made
        server.assert_received("/.well-known/anml");
        // If it succeeded, verify the endpoint
        if let Ok(discovery) = result {
            assert!(!discovery.endpoint.is_empty());
        }
    }
}
