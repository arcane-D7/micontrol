//! Optional BYOK suggestion adapter for OS Turbo.
//!
//! S39-007: the repository already has optional BYOK analysis. This module
//! defines the **only** shape a model suggestion may take, and the rules that
//! keep it advisory.
//!
//! Guarantees:
//!
//! 1. **Structured and bounded.** A suggestion is a closed struct of enums and
//!    small integers. Free text is limited to a short rationale that is
//!    length-capped and never executed.
//! 2. **Validated before use.** [`validate_suggestion`] rejects malformed,
//!    out-of-range, unallowlisted or privacy-violating suggestions.
//! 3. **User-approved.** A suggestion that could cause a write requires an
//!    explicit, current approval. Denial or expiry is a no-op.
//! 4. **Independent.** Nothing here can change deterministic statistics or
//!    trigger an autonomous action: the adapter returns a decision for the UI
//!    to present, and the policy loop never reads it.

use serde::{Deserialize, Serialize};

use super::os_turbo_policy::PolicyAction;

/// Maximum accepted rationale length (characters).
pub const MAX_RATIONALE_CHARS: usize = 280;

/// Maximum accepted suggestions in one response.
pub const MAX_SUGGESTIONS: usize = 5;

/// How long an approval remains valid (milliseconds).
pub const APPROVAL_TTL_MS: u64 = 15 * 60 * 1000;

/// A single model suggestion. Closed schema: no arbitrary fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The action the model proposes. Must be on the policy allowlist.
    pub action: PolicyAction,
    /// Model's stated confidence, 0–100. Advisory only.
    pub confidence_percent: u8,
    /// Short rationale. Length-capped, never executed, never logged verbatim.
    pub rationale: String,
}

/// A validated suggestion set returned to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestionSet {
    pub suggestions: Vec<Suggestion>,
    /// Whether the model response was accepted at all.
    pub accepted: bool,
    /// Safe rejection reason when `accepted == false`.
    pub rejection: Option<SuggestionRejection>,
}

/// Why a suggestion set was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionRejection {
    /// More suggestions than [`MAX_SUGGESTIONS`].
    TooManySuggestions,
    /// A rationale exceeded [`MAX_RATIONALE_CHARS`].
    RationaleTooLong,
    /// A confidence value was outside 0–100.
    ConfidenceOutOfRange,
    /// An action was not on the policy allowlist.
    ActionNotAllowlisted,
    /// The response could not be parsed into the schema.
    Malformed,
    /// The response contained data that must not be stored or forwarded.
    PrivacyViolation,
}

impl SuggestionRejection {
    /// Safe, stable reason text (no model output).
    pub fn reason(self) -> &'static str {
        match self {
            Self::TooManySuggestions => "too many suggestions",
            Self::RationaleTooLong => "rationale too long",
            Self::ConfidenceOutOfRange => "confidence out of range",
            Self::ActionNotAllowlisted => "action not allowlisted",
            Self::Malformed => "malformed suggestion payload",
            Self::PrivacyViolation => "suggestion contained disallowed data",
        }
    }
}

/// Substrings that must never appear in a suggestion. Their presence means the
/// model echoed local identifiers, which must not be stored or forwarded.
const PRIVACY_DENYLIST: &[&str] = &[
    ".exe", "c:\\", "\\\\", "/users/", "hkey_", "sid:", "s-1-5-", "mac:", "serial:", "token",
    "api key", "apikey", "bearer ", "password",
];

/// Validate a raw suggestion set.
///
/// Returns an accepted set, or a rejected set with a safe reason. The function
/// never panics and never returns model text in the rejection.
pub fn validate_suggestion_set(suggestions: Vec<Suggestion>) -> SuggestionSet {
    if suggestions.len() > MAX_SUGGESTIONS {
        return rejected(SuggestionRejection::TooManySuggestions);
    }

    for suggestion in &suggestions {
        if !suggestion.action.is_allowed() {
            return rejected(SuggestionRejection::ActionNotAllowlisted);
        }
        if suggestion.confidence_percent > 100 {
            return rejected(SuggestionRejection::ConfidenceOutOfRange);
        }
        if suggestion.rationale.chars().count() > MAX_RATIONALE_CHARS {
            return rejected(SuggestionRejection::RationaleTooLong);
        }
        let lowered = suggestion.rationale.to_lowercase();
        if PRIVACY_DENYLIST
            .iter()
            .any(|needle| lowered.contains(needle))
        {
            return rejected(SuggestionRejection::PrivacyViolation);
        }
    }

    SuggestionSet {
        suggestions,
        accepted: true,
        rejection: None,
    }
}

fn rejected(rejection: SuggestionRejection) -> SuggestionSet {
    SuggestionSet {
        suggestions: Vec::new(),
        accepted: false,
        rejection: Some(rejection),
    }
}

/// Parse a raw JSON string into a suggestion set, rejecting malformed input.
pub fn parse_suggestion_set(json: &str) -> SuggestionSet {
    match serde_json::from_str::<Vec<Suggestion>>(json) {
        Ok(suggestions) => validate_suggestion_set(suggestions),
        Err(_) => rejected(SuggestionRejection::Malformed),
    }
}

/// A user's explicit approval of one suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub action: PolicyAction,
    /// Milliseconds (UNIX epoch) when the user approved.
    pub approved_at_ms: u64,
}

/// Outcome of checking an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    /// The approval is current and matches the suggestion.
    Approved,
    /// No approval exists.
    Missing,
    /// The approval is older than [`APPROVAL_TTL_MS`].
    Expired,
    /// The approval is for a different action.
    ActionMismatch,
}

/// Check whether an approval authorizes a suggestion right now.
pub fn check_approval(
    approval: Option<&Approval>,
    action: PolicyAction,
    now_ms: u64,
) -> ApprovalOutcome {
    let Some(approval) = approval else {
        return ApprovalOutcome::Missing;
    };
    if approval.action != action {
        return ApprovalOutcome::ActionMismatch;
    }
    if now_ms.saturating_sub(approval.approved_at_ms) > APPROVAL_TTL_MS {
        return ApprovalOutcome::Expired;
    }
    ApprovalOutcome::Approved
}

/// What the UI should do with a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionDisposition {
    /// Present the suggestion and wait for explicit approval.
    AwaitApproval,
    /// The suggestion is approved and may be offered to the policy as a
    /// *proposal* (the policy still applies its own evidence gate).
    Approved,
    /// Do nothing.
    NoOp,
}

/// Decide the disposition of a suggestion. Never executes anything.
pub fn disposition(
    set: &SuggestionSet,
    action: PolicyAction,
    approval: Option<&Approval>,
    now_ms: u64,
) -> SuggestionDisposition {
    if !set.accepted {
        return SuggestionDisposition::NoOp;
    }
    if !set.suggestions.iter().any(|s| s.action == action) {
        return SuggestionDisposition::NoOp;
    }
    match check_approval(approval, action, now_ms) {
        ApprovalOutcome::Approved => SuggestionDisposition::Approved,
        _ => SuggestionDisposition::AwaitApproval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(action: PolicyAction, confidence: u8, rationale: &str) -> Suggestion {
        Suggestion {
            action,
            confidence_percent: confidence,
            rationale: rationale.to_string(),
        }
    }

    #[test]
    fn valid_suggestion_is_accepted() {
        let set = validate_suggestion_set(vec![suggestion(
            PolicyAction::ThrottleAllowlistedBackground,
            80,
            "Background CPU dropped in the last window.",
        )]);
        assert!(set.accepted);
        assert_eq!(set.suggestions.len(), 1);
        assert!(set.rejection.is_none());
    }

    #[test]
    fn too_many_suggestions_are_rejected() {
        let many = vec![
            suggestion(PolicyAction::ThrottleAllowlistedBackground, 50, "ok");
            MAX_SUGGESTIONS + 1
        ];
        let set = validate_suggestion_set(many);
        assert!(!set.accepted);
        assert_eq!(set.rejection, Some(SuggestionRejection::TooManySuggestions));
        assert!(set.suggestions.is_empty());
    }

    #[test]
    fn overlong_rationale_is_rejected() {
        let long = "a".repeat(MAX_RATIONALE_CHARS + 1);
        let set = validate_suggestion_set(vec![suggestion(
            PolicyAction::ThrottleAllowlistedBackground,
            50,
            &long,
        )]);
        assert_eq!(set.rejection, Some(SuggestionRejection::RationaleTooLong));
    }

    #[test]
    fn out_of_range_confidence_is_rejected() {
        let set = validate_suggestion_set(vec![suggestion(
            PolicyAction::ThrottleAllowlistedBackground,
            101,
            "ok",
        )]);
        assert_eq!(
            set.rejection,
            Some(SuggestionRejection::ConfidenceOutOfRange)
        );
    }

    #[test]
    fn privacy_violating_rationale_is_rejected() {
        for bad in [
            "throttle SearchIndexer.exe now",
            "path C:\\Program Files\\MiControl",
            "token abc123",
            "api key present",
            "SID: S-1-5-21-1",
        ] {
            let set = validate_suggestion_set(vec![suggestion(
                PolicyAction::ThrottleAllowlistedBackground,
                50,
                bad,
            )]);
            assert_eq!(
                set.rejection,
                Some(SuggestionRejection::PrivacyViolation),
                "should reject: {bad}"
            );
        }
    }

    #[test]
    fn malformed_json_is_rejected() {
        let set = parse_suggestion_set("{not json");
        assert!(!set.accepted);
        assert_eq!(set.rejection, Some(SuggestionRejection::Malformed));
    }

    #[test]
    fn unknown_action_in_json_is_rejected() {
        let json = r#"[{"action":"write_thermal_limit","confidence_percent":90,"rationale":"ok"}]"#;
        let set = parse_suggestion_set(json);
        assert!(!set.accepted);
        assert_eq!(set.rejection, Some(SuggestionRejection::Malformed));
    }

    #[test]
    fn valid_json_round_trips() {
        let json = r#"[{"action":"throttle_allowlisted_background","confidence_percent":75,"rationale":"Measured improvement."}]"#;
        let set = parse_suggestion_set(json);
        assert!(set.accepted);
        assert_eq!(set.suggestions.len(), 1);
        assert_eq!(
            set.suggestions[0].action,
            PolicyAction::ThrottleAllowlistedBackground
        );
    }

    #[test]
    fn missing_approval_awaits_user() {
        assert_eq!(
            check_approval(None, PolicyAction::ThrottleAllowlistedBackground, 1000),
            ApprovalOutcome::Missing
        );
    }

    #[test]
    fn expired_approval_is_rejected() {
        let approval = Approval {
            action: PolicyAction::ThrottleAllowlistedBackground,
            approved_at_ms: 1000,
        };
        assert_eq!(
            check_approval(
                Some(&approval),
                PolicyAction::ThrottleAllowlistedBackground,
                1000 + APPROVAL_TTL_MS + 1
            ),
            ApprovalOutcome::Expired
        );
    }

    #[test]
    fn approval_for_another_action_is_rejected() {
        let approval = Approval {
            action: PolicyAction::UseHighPerformanceScheme,
            approved_at_ms: 1000,
        };
        assert_eq!(
            check_approval(
                Some(&approval),
                PolicyAction::ThrottleAllowlistedBackground,
                1500
            ),
            ApprovalOutcome::ActionMismatch
        );
    }

    #[test]
    fn current_matching_approval_is_accepted() {
        let approval = Approval {
            action: PolicyAction::ThrottleAllowlistedBackground,
            approved_at_ms: 1000,
        };
        assert_eq!(
            check_approval(
                Some(&approval),
                PolicyAction::ThrottleAllowlistedBackground,
                1000 + APPROVAL_TTL_MS
            ),
            ApprovalOutcome::Approved
        );
    }

    #[test]
    fn disposition_awaits_approval_then_approves() {
        let set = validate_suggestion_set(vec![suggestion(
            PolicyAction::ThrottleAllowlistedBackground,
            80,
            "ok",
        )]);
        assert_eq!(
            disposition(
                &set,
                PolicyAction::ThrottleAllowlistedBackground,
                None,
                1000
            ),
            SuggestionDisposition::AwaitApproval
        );

        let approval = Approval {
            action: PolicyAction::ThrottleAllowlistedBackground,
            approved_at_ms: 1000,
        };
        assert_eq!(
            disposition(
                &set,
                PolicyAction::ThrottleAllowlistedBackground,
                Some(&approval),
                1500
            ),
            SuggestionDisposition::Approved
        );
    }

    #[test]
    fn rejected_set_is_always_a_no_op() {
        let set = rejected(SuggestionRejection::Malformed);
        let approval = Approval {
            action: PolicyAction::ThrottleAllowlistedBackground,
            approved_at_ms: 1000,
        };
        assert_eq!(
            disposition(
                &set,
                PolicyAction::ThrottleAllowlistedBackground,
                Some(&approval),
                1500
            ),
            SuggestionDisposition::NoOp
        );
    }

    #[test]
    fn suggestion_for_absent_action_is_a_no_op() {
        let set = validate_suggestion_set(vec![suggestion(
            PolicyAction::UseHighPerformanceScheme,
            80,
            "ok",
        )]);
        assert_eq!(
            disposition(
                &set,
                PolicyAction::ThrottleAllowlistedBackground,
                None,
                1000
            ),
            SuggestionDisposition::NoOp
        );
    }

    #[test]
    fn rejection_reasons_are_safe_and_stable() {
        for rejection in [
            SuggestionRejection::TooManySuggestions,
            SuggestionRejection::RationaleTooLong,
            SuggestionRejection::ConfidenceOutOfRange,
            SuggestionRejection::ActionNotAllowlisted,
            SuggestionRejection::Malformed,
            SuggestionRejection::PrivacyViolation,
        ] {
            let reason = rejection.reason();
            assert!(!reason.is_empty());
            assert!(!reason.contains(".exe"));
            assert!(!reason.contains("\\"));
        }
    }

    #[test]
    fn adapter_exposes_no_execution_path() {
        // Structural guarantee: the module's public surface returns only
        // validated data and dispositions. There is no function that applies
        // an action, so a model suggestion cannot trigger a write here.
        let set = validate_suggestion_set(vec![suggestion(
            PolicyAction::ThrottleAllowlistedBackground,
            99,
            "ok",
        )]);
        let approval = Approval {
            action: PolicyAction::ThrottleAllowlistedBackground,
            approved_at_ms: 1000,
        };
        // Even an approved suggestion yields only a disposition value.
        let result = disposition(
            &set,
            PolicyAction::ThrottleAllowlistedBackground,
            Some(&approval),
            1500,
        );
        assert_eq!(result, SuggestionDisposition::Approved);
    }
}
