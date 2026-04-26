//! Error types for the ANML client.
//!
//! Every variant carries enough context for the developer to understand what
//! went wrong without reaching for a debugger. Messages are single-line and
//! phrased as "expected X, got Y" or "field 'Z' requires W" so they read
//! well in structured logs.

use std::fmt;

/// Alias for `std::result::Result<T, AnmlClientError>`.
pub type Result<T> = std::result::Result<T, AnmlClientError>;

/// Errors that can occur during ANML client operations.
///
/// Each variant includes contextual fields so the developer can understand
/// what went wrong and take corrective action.
#[derive(Debug)]
#[non_exhaustive]
pub enum AnmlClientError {
    /// An HTTP transport error from `reqwest`.
    Http {
        /// The underlying reqwest error.
        source: reqwest::Error,
    },

    /// An ANML parse or validation error from the `anml` crate.
    Parse {
        /// The underlying anml error.
        source: anml::errors::AnmlError,
    },

    /// The document was fetched over plaintext HTTP but contains security-sensitive
    /// sections (`<constraints>`, `<interact>`, or `<ask requires="explicit">`).
    TransportInsecure {
        /// The URL that was fetched.
        url: String,
        /// Why the transport was rejected.
        reason: String,
    },

    /// The response `Content-Type` did not match `application/anml+xml`.
    ContentTypeMismatch {
        /// The expected content type.
        expected: String,
        /// The actual content type received.
        actual: String,
    },

    /// The service does not support the requested ANML version.
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:unsupported-version`.
    UnsupportedVersion {
        /// Human-readable detail from the problem response.
        detail: String,
        /// Versions the service does support.
        supported: Vec<String>,
    },

    /// The document declares a conformance profile the client does not implement.
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:unsupported-profile`.
    UnsupportedProfile {
        /// The profile URI that is not supported.
        profile_uri: String,
    },

    /// The document declares a required extension namespace the client does not recognize.
    UnsupportedExtension {
        /// The extension namespace URI.
        namespace_uri: String,
    },

    /// The document is not well-formed or fails ANML validity checks.
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:malformed-document`.
    MalformedDocument {
        /// Human-readable detail about the malformation.
        detail: String,
    },

    /// A multi-step flow was aborted (retry budget exhausted, no fallback).
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:flow-aborted`.
    FlowAborted {
        /// The step that failed.
        step_id: String,
        /// Why the flow was aborted.
        detail: String,
    },

    /// A subresource integrity check failed.
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:integrity-mismatch`.
    IntegrityMismatch {
        /// The element whose resource failed verification (e.g. `img`, `audio`).
        element: String,
        /// The expected digest.
        expected: String,
        /// The observed digest.
        observed: String,
    },

    /// A document exceeded an RFC resource limit.
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:resource-limit-exceeded`.
    ResourceLimitExceeded {
        /// Which limit was exceeded (e.g. `max_document_size`).
        limit: String,
        /// The actual value.
        value: u64,
        /// The configured maximum.
        max: u64,
    },

    /// Disclosure was denied because the principal did not grant consent.
    ConsentDenied {
        /// The field for which consent was denied.
        field: String,
        /// The governing disclosure rule.
        rule: String,
        /// The consent scope that was required.
        consent_scope: String,
    },

    /// The per-field rate limit was exceeded.
    RateLimited {
        /// The field that is rate-limited.
        field: String,
        /// Seconds until the next disclosure is allowed, if known.
        retry_after: Option<u64>,
    },

    /// The trust policy denied the request.
    ///
    /// Maps to `urn:ietf:params:xml:ns:anml:problem:trust-level-regression`
    /// when caused by a trust-level regression.
    TrustInsufficient {
        /// The origin that was denied.
        origin: String,
        /// Why trust was insufficient.
        reason: String,
    },

    /// The per-document action budget was exceeded.
    ActionBudgetExceeded {
        /// Which budget was exceeded (e.g. `max_requests`, `max_origins`).
        budget_type: String,
        /// The configured limit.
        limit: u32,
    },

    /// An action endpoint resolved to a private or loopback IP (SSRF protection).
    SsrfBlocked {
        /// The endpoint that was blocked.
        endpoint: String,
    },

    /// A flow step regressed to an earlier state unexpectedly.
    UnexpectedStateRegression {
        /// The step that regressed.
        step_id: String,
        /// The previous status.
        from: String,
        /// The new (regressed) status.
        to: String,
    },

    /// A parameter failed validation against its declared constraints.
    ParamValidation {
        /// The parameter name.
        param: String,
        /// The constraint that was violated (e.g. `pattern`, `min`, `required`).
        constraint: String,
        /// The expected value or constraint description.
        expected: String,
        /// The actual value provided.
        actual: String,
    },

    /// An operation timed out.
    Timeout {
        /// What timed out (e.g. `per_request`, `per_action`, `per_flow`, `parse`).
        operation: String,
        /// The timeout duration in seconds.
        timeout_secs: u64,
    },
}

impl fmt::Display for AnmlClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { source } => write!(f, "HTTP error: {source}"),
            Self::Parse { source } => write!(f, "ANML parse error: {source}"),
            Self::TransportInsecure { url, reason } => {
                write!(f, "transport insecure: {reason} (url: {url})")
            }
            Self::ContentTypeMismatch { expected, actual } => {
                write!(
                    f,
                    "content type mismatch: expected '{expected}', got '{actual}'"
                )
            }
            Self::UnsupportedVersion { detail, supported } => {
                write!(
                    f,
                    "unsupported version: {detail} (supported: {})",
                    supported.join(", ")
                )
            }
            Self::UnsupportedProfile { profile_uri } => {
                write!(f, "unsupported profile: '{profile_uri}'")
            }
            Self::UnsupportedExtension { namespace_uri } => {
                write!(
                    f,
                    "unsupported required extension: '{namespace_uri}'"
                )
            }
            Self::MalformedDocument { detail } => {
                write!(f, "malformed document: {detail}")
            }
            Self::FlowAborted { step_id, detail } => {
                write!(f, "flow aborted at step '{step_id}': {detail}")
            }
            Self::IntegrityMismatch {
                element,
                expected,
                observed,
            } => {
                write!(
                    f,
                    "integrity mismatch on <{element}>: expected '{expected}', got '{observed}'"
                )
            }
            Self::ResourceLimitExceeded { limit, value, max } => {
                write!(
                    f,
                    "resource limit exceeded: '{limit}' is {value}, max is {max}"
                )
            }
            Self::ConsentDenied {
                field,
                rule,
                consent_scope,
            } => {
                write!(
                    f,
                    "consent denied: field '{field}' requires {rule} (consent-scope={consent_scope})"
                )
            }
            Self::RateLimited { field, retry_after } => match retry_after {
                Some(secs) => write!(
                    f,
                    "rate limited: field '{field}', retry after {secs}s"
                ),
                None => write!(f, "rate limited: field '{field}'"),
            },
            Self::TrustInsufficient { origin, reason } => {
                write!(
                    f,
                    "trust insufficient for origin '{origin}': {reason}"
                )
            }
            Self::ActionBudgetExceeded { budget_type, limit } => {
                write!(
                    f,
                    "action budget exceeded: '{budget_type}' limit is {limit}"
                )
            }
            Self::SsrfBlocked { endpoint } => {
                write!(f, "SSRF blocked: endpoint '{endpoint}' resolves to a private address")
            }
            Self::UnexpectedStateRegression {
                step_id,
                from,
                to,
            } => {
                write!(
                    f,
                    "unexpected state regression: step '{step_id}' moved from '{from}' to '{to}'"
                )
            }
            Self::ParamValidation {
                param,
                constraint,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "param validation: '{param}' {constraint} expected {expected}, got '{actual}'"
                )
            }
            Self::Timeout {
                operation,
                timeout_secs,
            } => {
                write!(f, "timeout: {operation} exceeded {timeout_secs}s")
            }
        }
    }
}

impl std::error::Error for AnmlClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http { source } => Some(source),
            Self::Parse { source } => Some(source),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for AnmlClientError {
    fn from(err: reqwest::Error) -> Self {
        Self::Http { source: err }
    }
}

impl From<anml::errors::AnmlError> for AnmlClientError {
    fn from(err: anml::errors::AnmlError) -> Self {
        Self::Parse { source: err }
    }
}

/// RFC problem type URIs defined in the ANML 1.0 specification.
pub mod problem_types {
    /// `urn:ietf:params:xml:ns:anml:problem:unsupported-version`
    pub const UNSUPPORTED_VERSION: &str =
        "urn:ietf:params:xml:ns:anml:problem:unsupported-version";
    /// `urn:ietf:params:xml:ns:anml:problem:unsupported-profile`
    pub const UNSUPPORTED_PROFILE: &str =
        "urn:ietf:params:xml:ns:anml:problem:unsupported-profile";
    /// `urn:ietf:params:xml:ns:anml:problem:malformed-document`
    pub const MALFORMED_DOCUMENT: &str =
        "urn:ietf:params:xml:ns:anml:problem:malformed-document";
    /// `urn:ietf:params:xml:ns:anml:problem:flow-aborted`
    pub const FLOW_ABORTED: &str =
        "urn:ietf:params:xml:ns:anml:problem:flow-aborted";
    /// `urn:ietf:params:xml:ns:anml:problem:trust-level-regression`
    pub const TRUST_LEVEL_REGRESSION: &str =
        "urn:ietf:params:xml:ns:anml:problem:trust-level-regression";
    /// `urn:ietf:params:xml:ns:anml:problem:integrity-mismatch`
    pub const INTEGRITY_MISMATCH: &str =
        "urn:ietf:params:xml:ns:anml:problem:integrity-mismatch";
    /// `urn:ietf:params:xml:ns:anml:problem:resource-limit-exceeded`
    pub const RESOURCE_LIMIT_EXCEEDED: &str =
        "urn:ietf:params:xml:ns:anml:problem:resource-limit-exceeded";
}

/// Maps an RFC problem type URI to a descriptive variant name.
///
/// Returns `Some(variant_name)` for recognized URIs, `None` otherwise.
/// This is useful when parsing `<problem type="...">` elements from
/// service error responses.
pub fn problem_type_to_variant(uri: &str) -> Option<&'static str> {
    match uri {
        problem_types::UNSUPPORTED_VERSION => Some("UnsupportedVersion"),
        problem_types::UNSUPPORTED_PROFILE => Some("UnsupportedProfile"),
        problem_types::MALFORMED_DOCUMENT => Some("MalformedDocument"),
        problem_types::FLOW_ABORTED => Some("FlowAborted"),
        problem_types::TRUST_LEVEL_REGRESSION => Some("TrustInsufficient"),
        problem_types::INTEGRITY_MISMATCH => Some("IntegrityMismatch"),
        problem_types::RESOURCE_LIMIT_EXCEEDED => Some("ResourceLimitExceeded"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_http_error() {
        // We can't easily construct a reqwest::Error, so we test the other variants.
        // The Http variant is tested via the From impl below.
    }

    #[test]
    fn display_parse_error() {
        let anml_err = anml::errors::AnmlError::Parse(anml::errors::AnmlParseError {
            line: 10,
            column: 5,
            reason: "unexpected token".to_string(),
        });
        let err = AnmlClientError::from(anml_err);
        let msg = err.to_string();
        assert!(msg.contains("ANML parse error"), "got: {msg}");
        assert!(msg.contains("unexpected token"), "got: {msg}");
    }

    #[test]
    fn display_transport_insecure() {
        let err = AnmlClientError::TransportInsecure {
            url: "http://example.com/service".into(),
            reason: "document contains <constraints>".into(),
        };
        assert_eq!(
            err.to_string(),
            "transport insecure: document contains <constraints> (url: http://example.com/service)"
        );
    }

    #[test]
    fn display_content_type_mismatch() {
        let err = AnmlClientError::ContentTypeMismatch {
            expected: "application/anml+xml".into(),
            actual: "text/html".into(),
        };
        assert_eq!(
            err.to_string(),
            "content type mismatch: expected 'application/anml+xml', got 'text/html'"
        );
    }

    #[test]
    fn display_unsupported_version() {
        let err = AnmlClientError::UnsupportedVersion {
            detail: "version 2.0 not supported".into(),
            supported: vec!["1.0".into(), "1.1".into()],
        };
        assert_eq!(
            err.to_string(),
            "unsupported version: version 2.0 not supported (supported: 1.0, 1.1)"
        );
    }

    #[test]
    fn display_unsupported_profile() {
        let err = AnmlClientError::UnsupportedProfile {
            profile_uri: "urn:ietf:anml:profile:signed-answer-1.0".into(),
        };
        assert_eq!(
            err.to_string(),
            "unsupported profile: 'urn:ietf:anml:profile:signed-answer-1.0'"
        );
    }

    #[test]
    fn display_unsupported_extension() {
        let err = AnmlClientError::UnsupportedExtension {
            namespace_uri: "https://example.com/anml/ext/payments/1".into(),
        };
        assert_eq!(
            err.to_string(),
            "unsupported required extension: 'https://example.com/anml/ext/payments/1'"
        );
    }

    #[test]
    fn display_malformed_document() {
        let err = AnmlClientError::MalformedDocument {
            detail: "missing root <anml> element".into(),
        };
        assert_eq!(
            err.to_string(),
            "malformed document: missing root <anml> element"
        );
    }

    #[test]
    fn display_flow_aborted() {
        let err = AnmlClientError::FlowAborted {
            step_id: "payment".into(),
            detail: "retry budget exhausted".into(),
        };
        assert_eq!(
            err.to_string(),
            "flow aborted at step 'payment': retry budget exhausted"
        );
    }

    #[test]
    fn display_integrity_mismatch() {
        let err = AnmlClientError::IntegrityMismatch {
            element: "img".into(),
            expected: "sha256-abc123".into(),
            observed: "sha256-def456".into(),
        };
        assert_eq!(
            err.to_string(),
            "integrity mismatch on <img>: expected 'sha256-abc123', got 'sha256-def456'"
        );
    }

    #[test]
    fn display_resource_limit_exceeded() {
        let err = AnmlClientError::ResourceLimitExceeded {
            limit: "max_document_size".into(),
            value: 2_000_000,
            max: 1_048_576,
        };
        assert_eq!(
            err.to_string(),
            "resource limit exceeded: 'max_document_size' is 2000000, max is 1048576"
        );
    }

    #[test]
    fn display_consent_denied() {
        let err = AnmlClientError::ConsentDenied {
            field: "email".into(),
            rule: "explicit".into(),
            consent_scope: "session".into(),
        };
        assert_eq!(
            err.to_string(),
            "consent denied: field 'email' requires explicit (consent-scope=session)"
        );
    }

    #[test]
    fn display_rate_limited_with_retry() {
        let err = AnmlClientError::RateLimited {
            field: "phone".into(),
            retry_after: Some(3600),
        };
        assert_eq!(
            err.to_string(),
            "rate limited: field 'phone', retry after 3600s"
        );
    }

    #[test]
    fn display_rate_limited_without_retry() {
        let err = AnmlClientError::RateLimited {
            field: "phone".into(),
            retry_after: None,
        };
        assert_eq!(err.to_string(), "rate limited: field 'phone'");
    }

    #[test]
    fn display_trust_insufficient() {
        let err = AnmlClientError::TrustInsufficient {
            origin: "https://evil.example.com".into(),
            reason: "origin not in allow list".into(),
        };
        assert_eq!(
            err.to_string(),
            "trust insufficient for origin 'https://evil.example.com': origin not in allow list"
        );
    }

    #[test]
    fn display_action_budget_exceeded() {
        let err = AnmlClientError::ActionBudgetExceeded {
            budget_type: "max_requests".into(),
            limit: 50,
        };
        assert_eq!(
            err.to_string(),
            "action budget exceeded: 'max_requests' limit is 50"
        );
    }

    #[test]
    fn display_ssrf_blocked() {
        let err = AnmlClientError::SsrfBlocked {
            endpoint: "http://127.0.0.1:8080/internal".into(),
        };
        assert_eq!(
            err.to_string(),
            "SSRF blocked: endpoint 'http://127.0.0.1:8080/internal' resolves to a private address"
        );
    }

    #[test]
    fn display_unexpected_state_regression() {
        let err = AnmlClientError::UnexpectedStateRegression {
            step_id: "confirm".into(),
            from: "completed".into(),
            to: "pending".into(),
        };
        assert_eq!(
            err.to_string(),
            "unexpected state regression: step 'confirm' moved from 'completed' to 'pending'"
        );
    }

    #[test]
    fn display_param_validation() {
        let err = AnmlClientError::ParamValidation {
            param: "reason".into(),
            constraint: "one-of".into(),
            expected: "[damaged, wrong-item, not-needed]".into(),
            actual: "broken".into(),
        };
        assert_eq!(
            err.to_string(),
            "param validation: 'reason' one-of expected [damaged, wrong-item, not-needed], got 'broken'"
        );
    }

    #[test]
    fn display_timeout() {
        let err = AnmlClientError::Timeout {
            operation: "per_request".into(),
            timeout_secs: 30,
        };
        assert_eq!(err.to_string(), "timeout: per_request exceeded 30s");
    }

    #[test]
    fn from_anml_error() {
        let anml_err = anml::errors::AnmlError::Parse(anml::errors::AnmlParseError {
            line: 1,
            column: 1,
            reason: "bad xml".to_string(),
        });
        let err: AnmlClientError = anml_err.into();
        assert!(matches!(err, AnmlClientError::Parse { .. }));
    }

    #[test]
    fn error_source_for_parse() {
        let anml_err = anml::errors::AnmlError::Parse(anml::errors::AnmlParseError {
            line: 1,
            column: 1,
            reason: "test".to_string(),
        });
        let err = AnmlClientError::from(anml_err);
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn error_source_for_non_wrapped() {
        let err = AnmlClientError::Timeout {
            operation: "test".into(),
            timeout_secs: 1,
        };
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn problem_type_mapping() {
        assert_eq!(
            problem_type_to_variant(problem_types::UNSUPPORTED_VERSION),
            Some("UnsupportedVersion")
        );
        assert_eq!(
            problem_type_to_variant(problem_types::UNSUPPORTED_PROFILE),
            Some("UnsupportedProfile")
        );
        assert_eq!(
            problem_type_to_variant(problem_types::MALFORMED_DOCUMENT),
            Some("MalformedDocument")
        );
        assert_eq!(
            problem_type_to_variant(problem_types::FLOW_ABORTED),
            Some("FlowAborted")
        );
        assert_eq!(
            problem_type_to_variant(problem_types::TRUST_LEVEL_REGRESSION),
            Some("TrustInsufficient")
        );
        assert_eq!(
            problem_type_to_variant(problem_types::INTEGRITY_MISMATCH),
            Some("IntegrityMismatch")
        );
        assert_eq!(
            problem_type_to_variant(problem_types::RESOURCE_LIMIT_EXCEEDED),
            Some("ResourceLimitExceeded")
        );
        assert_eq!(problem_type_to_variant("about:blank"), None);
        assert_eq!(problem_type_to_variant("urn:unknown"), None);
    }

    #[test]
    fn result_type_alias_works() {
        fn returns_ok() -> Result<u32> {
            Ok(42)
        }
        fn returns_err() -> Result<u32> {
            Err(AnmlClientError::Timeout {
                operation: "test".into(),
                timeout_secs: 1,
            })
        }
        assert!(returns_ok().is_ok());
        assert!(returns_err().is_err());
    }

    #[test]
    fn error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AnmlClientError>();
    }

    #[test]
    fn non_exhaustive_allows_match_with_wildcard() {
        let err = AnmlClientError::Timeout {
            operation: "test".into(),
            timeout_secs: 1,
        };
        // This match must compile with a wildcard arm due to #[non_exhaustive]
        match err {
            AnmlClientError::Timeout { .. } => {}
            _ => {}
        }
    }

    #[test]
    fn display_messages_are_single_line() {
        let errors: Vec<AnmlClientError> = vec![
            AnmlClientError::TransportInsecure {
                url: "http://x.com".into(),
                reason: "test".into(),
            },
            AnmlClientError::ContentTypeMismatch {
                expected: "a".into(),
                actual: "b".into(),
            },
            AnmlClientError::UnsupportedVersion {
                detail: "v2".into(),
                supported: vec!["1.0".into()],
            },
            AnmlClientError::UnsupportedProfile {
                profile_uri: "urn:test".into(),
            },
            AnmlClientError::UnsupportedExtension {
                namespace_uri: "urn:ext".into(),
            },
            AnmlClientError::MalformedDocument {
                detail: "bad".into(),
            },
            AnmlClientError::FlowAborted {
                step_id: "s1".into(),
                detail: "fail".into(),
            },
            AnmlClientError::IntegrityMismatch {
                element: "img".into(),
                expected: "a".into(),
                observed: "b".into(),
            },
            AnmlClientError::ResourceLimitExceeded {
                limit: "size".into(),
                value: 2,
                max: 1,
            },
            AnmlClientError::ConsentDenied {
                field: "f".into(),
                rule: "explicit".into(),
                consent_scope: "session".into(),
            },
            AnmlClientError::RateLimited {
                field: "f".into(),
                retry_after: Some(60),
            },
            AnmlClientError::TrustInsufficient {
                origin: "o".into(),
                reason: "r".into(),
            },
            AnmlClientError::ActionBudgetExceeded {
                budget_type: "t".into(),
                limit: 1,
            },
            AnmlClientError::SsrfBlocked {
                endpoint: "e".into(),
            },
            AnmlClientError::UnexpectedStateRegression {
                step_id: "s".into(),
                from: "a".into(),
                to: "b".into(),
            },
            AnmlClientError::ParamValidation {
                param: "p".into(),
                constraint: "c".into(),
                expected: "e".into(),
                actual: "a".into(),
            },
            AnmlClientError::Timeout {
                operation: "op".into(),
                timeout_secs: 1,
            },
        ];
        for err in &errors {
            let msg = err.to_string();
            assert!(
                !msg.contains('\n'),
                "Display message must be single-line, got: {msg}"
            );
        }
    }
}
