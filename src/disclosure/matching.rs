//! Disclosure rule matching with RFC precedence.
//!
//! Implements the matching algorithm from RFC §8.5 step 1:
//! exact `field` > longest `field-prefix` > fewest metacharacters `field-pattern` > `default`.
//! Ties at the same precedence level are broken by document order (first wins).

use std::fmt;

// ---------------------------------------------------------------------------
// DisclosureRule — extended client-side disclosure rule
// ---------------------------------------------------------------------------

/// The kind of field selector on a `<disclosure>` rule.
///
/// A `<disclosure>` MUST carry exactly one of `field`, `field-prefix`, or
/// `field-pattern`. Mutual exclusion is enforced at construction via
/// [`DisclosureRule::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldSelector {
    /// Exact field name match.
    Exact(String),
    /// Prefix match: `S` matches field names equal to `S` or starting with `S.`.
    Prefix(String),
    /// Glob pattern match with `*`, `**`, `?` metacharacters.
    Pattern(String),
    /// Default rule (matches any field not matched by a more specific rule).
    Default,
}

/// Consent scope for a disclosure rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConsentScope {
    /// Consent applies only to the current interaction.
    Session,
    /// Consent applies to all interactions with the same origin.
    Origin,
    /// Consent applies across origins.
    Global,
}

impl Default for ConsentScope {
    fn default() -> Self {
        Self::Session
    }
}

impl fmt::Display for ConsentScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session => write!(f, "session"),
            Self::Origin => write!(f, "origin"),
            Self::Global => write!(f, "global"),
        }
    }
}

/// An extended disclosure rule used by the client-side evaluation engine.
///
/// The `anml` crate's `AnmlDisclosure` only carries `field` and `requires`.
/// This struct adds the additional attributes needed for the full RFC
/// disclosure algorithm: `consent-scope`, `rate-limit`, `tokenize`, `purpose`,
/// and the extended field selectors (`field-prefix`, `field-pattern`, `default`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DisclosureRule {
    /// The field selector (exact, prefix, pattern, or default).
    pub selector: FieldSelector,
    /// The disclosure requirement (explicit, implicit, authentication, none).
    pub requires: anml::types::enums::DisclosureRequires,
    /// Consent scope (session, origin, global). Default: session.
    pub consent_scope: ConsentScope,
    /// Optional rate limit (max disclosures per 24h window).
    pub rate_limit: Option<u32>,
    /// Whether to tokenize the value before disclosure.
    pub tokenize: bool,
    /// Human-readable purpose description.
    pub purpose: Option<String>,
    /// Document order index (0-based) for tie-breaking.
    pub document_order: usize,
}

impl DisclosureRule {
    /// Validate mutual exclusion: a rule must have exactly one selector kind.
    /// This is enforced at construction; this method is for external validation
    /// of raw attribute combinations.
    pub fn validate_mutual_exclusion(
        has_field: bool,
        has_prefix: bool,
        has_pattern: bool,
    ) -> Result<(), String> {
        let count = has_field as u8 + has_prefix as u8 + has_pattern as u8;
        if count > 1 {
            return Err(
                "<disclosure> MUST NOT carry both field and field-prefix, \
                 nor field and field-pattern, nor field-prefix and field-pattern"
                    .to_string(),
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Matching logic
// ---------------------------------------------------------------------------

/// The result of matching a field name against a set of disclosure rules.
#[derive(Clone, Debug)]
pub struct MatchResult<'a> {
    /// The matched rule, or `None` if no rule matched.
    pub rule: Option<&'a DisclosureRule>,
    /// Whether the match was synthesized (no rule found → default explicit/session).
    pub synthesized: bool,
}

/// Match a field name against a set of disclosure rules using RFC precedence.
///
/// Precedence (highest to lowest):
/// 1. Exact `field` match
/// 2. `field-prefix` match — longest prefix wins
/// 3. `field-pattern` match — fewest metacharacters wins; ties by longest
///    literal prefix before first metacharacter; further ties by document order
/// 4. `default="true"` rule
///
/// Ties at the same level are broken by document order (first wins).
pub fn resolve_rule<'a>(field: &str, rules: &'a [DisclosureRule]) -> MatchResult<'a> {
    // 1. Exact match (first in document order)
    if let Some(rule) = rules.iter().find(|r| matches!(&r.selector, FieldSelector::Exact(f) if f == field)) {
        return MatchResult { rule: Some(rule), synthesized: false };
    }

    // 2. Prefix match — longest prefix wins, ties by document order
    let mut best_prefix: Option<&DisclosureRule> = None;
    let mut best_prefix_len: usize = 0;
    for rule in rules {
        if let FieldSelector::Prefix(ref prefix) = rule.selector {
            if matches_prefix(field, prefix) {
                let plen = prefix.len();
                if plen > best_prefix_len
                    || (plen == best_prefix_len
                        && best_prefix.map_or(true, |bp| rule.document_order < bp.document_order))
                {
                    best_prefix = Some(rule);
                    best_prefix_len = plen;
                }
            }
        }
    }
    if let Some(rule) = best_prefix {
        return MatchResult { rule: Some(rule), synthesized: false };
    }

    // 3. Pattern match — fewest metacharacters wins, then longest literal prefix, then document order
    let mut best_pattern: Option<&DisclosureRule> = None;
    let mut best_meta_count = usize::MAX;
    let mut best_literal_prefix_len: usize = 0;
    for rule in rules {
        if let FieldSelector::Pattern(ref pattern) = rule.selector {
            if matches_pattern(field, pattern) {
                let meta_count = count_metacharacters(pattern);
                let lit_prefix = literal_prefix_length(pattern);
                if meta_count < best_meta_count
                    || (meta_count == best_meta_count && lit_prefix > best_literal_prefix_len)
                    || (meta_count == best_meta_count
                        && lit_prefix == best_literal_prefix_len
                        && best_pattern
                            .map_or(true, |bp| rule.document_order < bp.document_order))
                {
                    best_pattern = Some(rule);
                    best_meta_count = meta_count;
                    best_literal_prefix_len = lit_prefix;
                }
            }
        }
    }
    if let Some(rule) = best_pattern {
        return MatchResult { rule: Some(rule), synthesized: false };
    }

    // 4. Default rule (first in document order)
    if let Some(rule) = rules.iter().find(|r| matches!(&r.selector, FieldSelector::Default)) {
        return MatchResult { rule: Some(rule), synthesized: false };
    }

    // 5. No match — caller should synthesize requires="explicit" + consent-scope="session"
    MatchResult { rule: None, synthesized: true }
}

// ---------------------------------------------------------------------------
// Field-prefix matching
// ---------------------------------------------------------------------------

/// Check if `field` matches the prefix `S`.
///
/// `S` matches field names that are exactly `S` or start with `S` followed
/// by `.`. For example, `contact` matches `contact`, `contact.email`,
/// `contact.phone.home`, but NOT `contacts`.
pub fn matches_prefix(field: &str, prefix: &str) -> bool {
    if field == prefix {
        return true;
    }
    if field.len() > prefix.len() {
        let rest = &field[prefix.len()..];
        return rest.starts_with('.');
    }
    false
}

// ---------------------------------------------------------------------------
// Glob pattern matching for field-pattern
// ---------------------------------------------------------------------------

/// Check if `field` matches the glob `pattern`.
///
/// Supported metacharacters:
/// - `*`  — zero or more characters except `.`
/// - `**` — zero or more characters including `.`
/// - `?`  — exactly one character except `.`
/// - `\X` — literal character X (backslash escape)
pub fn matches_pattern(field: &str, pattern: &str) -> bool {
    let pat_chars: Vec<PatternToken> = tokenize_pattern(pattern);
    pattern_match(&pat_chars, field.as_bytes(), 0, 0)
}

/// Count the number of metacharacters in a pattern (for precedence ranking).
pub fn count_metacharacters(pattern: &str) -> usize {
    let tokens = tokenize_pattern(pattern);
    tokens
        .iter()
        .filter(|t| matches!(t, PatternToken::Star | PatternToken::DoubleStar | PatternToken::Question))
        .count()
}

/// Length of the literal prefix before the first metacharacter.
pub fn literal_prefix_length(pattern: &str) -> usize {
    let mut len = 0;
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'*' | b'?' => break,
            b'\\' => {
                // escaped char counts as literal
                len += 1;
                i += 2;
            }
            _ => {
                len += 1;
                i += 1;
            }
        }
    }
    len
}

// ---------------------------------------------------------------------------
// Pattern tokenizer and recursive matcher
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum PatternToken {
    Literal(u8),
    Star,       // * — 0+ chars except '.'
    DoubleStar, // ** — 0+ chars including '.'
    Question,   // ? — exactly 1 char except '.'
}

fn tokenize_pattern(pattern: &str) -> Vec<PatternToken> {
    let bytes = pattern.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                tokens.push(PatternToken::Literal(bytes[i + 1]));
                i += 2;
            }
            b'*' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                tokens.push(PatternToken::DoubleStar);
                i += 2;
            }
            b'*' => {
                tokens.push(PatternToken::Star);
                i += 1;
            }
            b'?' => {
                tokens.push(PatternToken::Question);
                i += 1;
            }
            ch => {
                tokens.push(PatternToken::Literal(ch));
                i += 1;
            }
        }
    }
    tokens
}

fn pattern_match(tokens: &[PatternToken], input: &[u8], ti: usize, ii: usize) -> bool {
    if ti == tokens.len() {
        return ii == input.len();
    }

    match &tokens[ti] {
        PatternToken::Literal(ch) => {
            if ii < input.len() && input[ii] == *ch {
                pattern_match(tokens, input, ti + 1, ii + 1)
            } else {
                false
            }
        }
        PatternToken::Question => {
            // Exactly one char except '.'
            if ii < input.len() && input[ii] != b'.' {
                pattern_match(tokens, input, ti + 1, ii + 1)
            } else {
                false
            }
        }
        PatternToken::Star => {
            // Zero or more chars except '.'
            // Try consuming 0, 1, 2, ... chars (stopping at '.' or end)
            let mut j = ii;
            if pattern_match(tokens, input, ti + 1, j) {
                return true;
            }
            while j < input.len() && input[j] != b'.' {
                j += 1;
                if pattern_match(tokens, input, ti + 1, j) {
                    return true;
                }
            }
            false
        }
        PatternToken::DoubleStar => {
            // Zero or more chars including '.'
            for j in ii..=input.len() {
                if pattern_match(tokens, input, ti + 1, j) {
                    return true;
                }
            }
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use anml::types::enums::DisclosureRequires;

    fn make_rule(selector: FieldSelector, order: usize) -> DisclosureRule {
        DisclosureRule {
            selector,
            requires: DisclosureRequires::ExplicitConsent,
            consent_scope: ConsentScope::Session,
            rate_limit: None,
            tokenize: false,
            purpose: None,
            document_order: order,
        }
    }

    // -- Prefix matching --

    #[test]
    fn prefix_exact_match() {
        assert!(matches_prefix("contact", "contact"));
    }

    #[test]
    fn prefix_dot_child() {
        assert!(matches_prefix("contact.email", "contact"));
    }

    #[test]
    fn prefix_deep_child() {
        assert!(matches_prefix("contact.phone.home", "contact"));
    }

    #[test]
    fn prefix_no_match_plural() {
        assert!(!matches_prefix("contacts", "contact"));
    }

    #[test]
    fn prefix_no_match_shorter() {
        assert!(!matches_prefix("con", "contact"));
    }

    // -- Pattern matching --

    #[test]
    fn pattern_star_matches_segment() {
        assert!(matches_pattern("contact", "*"));
        assert!(matches_pattern("email", "*"));
        assert!(!matches_pattern("contact.email", "*"));
    }

    #[test]
    fn pattern_double_star_matches_all() {
        assert!(matches_pattern("contact", "**"));
        assert!(matches_pattern("contact.email", "**"));
        assert!(matches_pattern("a.b.c.d", "**"));
    }

    #[test]
    fn pattern_question_mark() {
        assert!(matches_pattern("ab", "a?"));
        assert!(!matches_pattern("a.", "a?"));
        assert!(!matches_pattern("a", "a?"));
    }

    #[test]
    fn pattern_star_in_segment() {
        assert!(matches_pattern("contact.email", "contact.*"));
        assert!(matches_pattern("contact.phone", "contact.*"));
        assert!(!matches_pattern("contact.phone.home", "contact.*"));
    }

    #[test]
    fn pattern_double_star_suffix() {
        assert!(matches_pattern("contact.email", "contact.**"));
        assert!(matches_pattern("contact.phone.home", "contact.**"));
        // "contact" alone does NOT match "contact.**" because the literal "." is required
        assert!(!matches_pattern("contact", "contact.**"));
    }

    #[test]
    fn pattern_escaped_star() {
        assert!(matches_pattern("a*b", "a\\*b"));
        assert!(!matches_pattern("axb", "a\\*b"));
    }

    #[test]
    fn pattern_escaped_question() {
        assert!(matches_pattern("a?b", "a\\?b"));
        assert!(!matches_pattern("axb", "a\\?b"));
    }

    // -- Metacharacter counting --

    #[test]
    fn count_meta_simple() {
        assert_eq!(count_metacharacters("contact.*"), 1);
        assert_eq!(count_metacharacters("**"), 1);
        assert_eq!(count_metacharacters("a.?.b.*"), 2);
        assert_eq!(count_metacharacters("a\\*b"), 0);
    }

    // -- Literal prefix length --

    #[test]
    fn literal_prefix_len() {
        assert_eq!(literal_prefix_length("contact.*"), 8); // "contact."
        assert_eq!(literal_prefix_length("*"), 0);
        assert_eq!(literal_prefix_length("a.b.*"), 4); // "a.b."
        assert_eq!(literal_prefix_length("a\\*b.*"), 4); // "a", escaped "*", "b", "."
    }

    // -- Resolve rule precedence --

    #[test]
    fn exact_wins_over_prefix() {
        let rules = vec![
            make_rule(FieldSelector::Prefix("contact".into()), 0),
            make_rule(FieldSelector::Exact("contact.email".into()), 1),
        ];
        let result = resolve_rule("contact.email", &rules);
        assert!(!result.synthesized);
        assert_eq!(
            result.rule.unwrap().selector,
            FieldSelector::Exact("contact.email".into())
        );
    }

    #[test]
    fn exact_wins_over_pattern() {
        let rules = vec![
            make_rule(FieldSelector::Pattern("contact.*".into()), 0),
            make_rule(FieldSelector::Exact("contact.email".into()), 1),
        ];
        let result = resolve_rule("contact.email", &rules);
        assert_eq!(
            result.rule.unwrap().selector,
            FieldSelector::Exact("contact.email".into())
        );
    }

    #[test]
    fn prefix_wins_over_pattern() {
        let rules = vec![
            make_rule(FieldSelector::Pattern("contact.*".into()), 0),
            make_rule(FieldSelector::Prefix("contact".into()), 1),
        ];
        let result = resolve_rule("contact.email", &rules);
        assert_eq!(
            result.rule.unwrap().selector,
            FieldSelector::Prefix("contact".into())
        );
    }

    #[test]
    fn longest_prefix_wins() {
        let rules = vec![
            make_rule(FieldSelector::Prefix("contact".into()), 0),
            make_rule(FieldSelector::Prefix("contact.phone".into()), 1),
        ];
        let result = resolve_rule("contact.phone.home", &rules);
        assert_eq!(
            result.rule.unwrap().selector,
            FieldSelector::Prefix("contact.phone".into())
        );
    }

    #[test]
    fn fewest_metacharacters_wins() {
        let rules = vec![
            make_rule(FieldSelector::Pattern("**".into()), 0),
            make_rule(FieldSelector::Pattern("contact.*".into()), 1),
        ];
        let result = resolve_rule("contact.email", &rules);
        assert_eq!(
            result.rule.unwrap().selector,
            FieldSelector::Pattern("contact.*".into())
        );
    }

    #[test]
    fn default_is_last_resort() {
        let rules = vec![
            make_rule(FieldSelector::Default, 0),
            make_rule(FieldSelector::Exact("other".into()), 1),
        ];
        let result = resolve_rule("unknown", &rules);
        assert_eq!(result.rule.unwrap().selector, FieldSelector::Default);
    }

    #[test]
    fn no_match_synthesizes() {
        let rules = vec![make_rule(FieldSelector::Exact("other".into()), 0)];
        let result = resolve_rule("unknown", &rules);
        assert!(result.synthesized);
        assert!(result.rule.is_none());
    }

    #[test]
    fn document_order_breaks_ties() {
        let mut rule0 = make_rule(FieldSelector::Exact("email".into()), 0);
        rule0.requires = DisclosureRequires::None;
        let mut rule1 = make_rule(FieldSelector::Exact("email".into()), 1);
        rule1.requires = DisclosureRequires::ExplicitConsent;
        let rules = vec![rule0, rule1];
        let result = resolve_rule("email", &rules);
        // First in document order wins
        assert_eq!(result.rule.unwrap().requires, DisclosureRequires::None);
    }

    // -- Mutual exclusion validation --

    #[test]
    fn mutual_exclusion_ok_single() {
        assert!(DisclosureRule::validate_mutual_exclusion(true, false, false).is_ok());
        assert!(DisclosureRule::validate_mutual_exclusion(false, true, false).is_ok());
        assert!(DisclosureRule::validate_mutual_exclusion(false, false, true).is_ok());
        assert!(DisclosureRule::validate_mutual_exclusion(false, false, false).is_ok());
    }

    #[test]
    fn mutual_exclusion_rejects_pairs() {
        assert!(DisclosureRule::validate_mutual_exclusion(true, true, false).is_err());
        assert!(DisclosureRule::validate_mutual_exclusion(true, false, true).is_err());
        assert!(DisclosureRule::validate_mutual_exclusion(false, true, true).is_err());
        assert!(DisclosureRule::validate_mutual_exclusion(true, true, true).is_err());
    }
}
