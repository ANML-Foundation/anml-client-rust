//! DNS-SD discovery (feature-gated: `dns-sd`).
//!
//! Per RFC Section 6.4.4, a domain MAY publish SRV records at
//! `_anml._tcp.{domain}` and TXT records with key `v=anml1` and optional
//! `path` key.
//!
//! This module uses `hickory-resolver` (pure Rust) to resolve these records
//! and construct a [`DiscoveryResult`].
//!
//! Requires the `dns-sd` feature flag.

use std::collections::HashMap;

use hickory_resolver::Resolver;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;

use crate::discovery::{DiscoveryMechanism, DiscoveryResult};
use crate::error::AnmlClientError;

/// The SRV record prefix for ANML service discovery.
const SRV_PREFIX: &str = "_anml._tcp";

/// The expected TXT record version key.
const TXT_VERSION_KEY: &str = "v";

/// The expected TXT record version value.
const TXT_VERSION_VALUE: &str = "anml1";

/// The TXT record key for the service path.
const TXT_PATH_KEY: &str = "path";

/// Attempt discovery via DNS-SD.
///
/// Resolves `_anml._tcp.{domain}` SRV records and associated TXT records.
/// Extracts the target host, port, and optional path from the records.
///
/// The `origin` should be a scheme+host (e.g. `"https://example.com"`).
/// The domain is extracted from the origin for DNS queries.
pub async fn discover_dns_sd(origin: &str) -> crate::Result<DiscoveryResult> {
    let parsed = url::Url::parse(origin).map_err(|e| AnmlClientError::MalformedDocument {
        detail: format!("invalid origin URL '{}': {}", origin, e),
    })?;

    let domain = parsed.host_str().ok_or_else(|| AnmlClientError::MalformedDocument {
        detail: format!("no host in origin '{}'", origin),
    })?;

    let scheme = parsed.scheme();
    let srv_name = format!("{}.{}.", SRV_PREFIX, domain);

    // Create a resolver with system defaults
    let resolver = Resolver::builder_with_config(
        ResolverConfig::default(),
        TokioConnectionProvider::default(),
    )
    .with_options(ResolverOpts::default())
    .build();

    // Resolve SRV records
    let srv_response = resolver
        .srv_lookup(&srv_name)
        .await
        .map_err(|e| AnmlClientError::MalformedDocument {
            detail: format!("DNS SRV lookup for '{}' failed: {}", srv_name, e),
        })?;

    let srv_record = srv_response
        .iter()
        .next()
        .ok_or_else(|| AnmlClientError::MalformedDocument {
            detail: format!("no SRV records found for '{}'", srv_name),
        })?;

    let target = srv_record.target().to_string();
    let target = target.trim_end_matches('.');
    let port = srv_record.port();

    // Try to resolve TXT records for metadata
    let txt_name = format!("{}.{}.", SRV_PREFIX, domain);
    let mut metadata = HashMap::new();
    let mut path = String::new();
    let mut found_version = false;

    if let Ok(txt_response) = resolver.txt_lookup(&txt_name).await {
        for txt_record in txt_response.iter() {
            let txt_data = txt_record.to_string();
            // TXT records contain key=value pairs
            for entry in txt_data.split(' ') {
                if let Some((key, value)) = entry.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();
                    if key == TXT_VERSION_KEY && value == TXT_VERSION_VALUE {
                        found_version = true;
                    }
                    if key == TXT_PATH_KEY {
                        path = value.to_string();
                    }
                    metadata.insert(key.to_string(), value.to_string());
                }
            }
        }
    }

    if !found_version {
        return Err(AnmlClientError::MalformedDocument {
            detail: format!(
                "DNS TXT records for '{}' do not contain '{}={}'",
                txt_name, TXT_VERSION_KEY, TXT_VERSION_VALUE
            ),
        });
    }

    // Construct the endpoint URL
    let endpoint = if path.is_empty() {
        format!("{}://{}:{}", scheme, target, port)
    } else {
        let path = if path.starts_with('/') {
            path
        } else {
            format!("/{}", path)
        };
        format!("{}://{}:{}{}", scheme, target, port, path)
    };

    Ok(DiscoveryResult {
        endpoint,
        version: Some("1.0".to_string()),
        metadata,
        mechanism: DiscoveryMechanism::DnsSd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srv_name_construction() {
        let domain = "example.com";
        let srv_name = format!("{}.{}.", SRV_PREFIX, domain);
        assert_eq!(srv_name, "_anml._tcp.example.com.");
    }

    #[test]
    fn srv_name_with_subdomain() {
        let domain = "api.example.com";
        let srv_name = format!("{}.{}.", SRV_PREFIX, domain);
        assert_eq!(srv_name, "_anml._tcp.api.example.com.");
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(SRV_PREFIX, "_anml._tcp");
        assert_eq!(TXT_VERSION_KEY, "v");
        assert_eq!(TXT_VERSION_VALUE, "anml1");
        assert_eq!(TXT_PATH_KEY, "path");
    }
}
