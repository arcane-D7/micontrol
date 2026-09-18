//! Bounded, allowlisted adaptive policy for OS Turbo.
//!
//! S39-005: measured evidence (S39-004) is not safe to apply directly. This
//! module turns evidence into a **bounded, reversible decision** and refuses
//! everything else.
//!
//! Hard guarantees enforced here:
//!
//! 1. **Allowlist only.** The policy can only emit actions from
//!    [`PolicyAction::ALL`]. Anything else is rejected by
//!    [`PolicyAction::is_allowed`].
//! 2. **No hardware writes.** OS Turbo is a scheduler/power-plan feature. The
//!    policy has no representation for thermal limits, EC registers, firmware
//!    settings, fan curves or TDP — [`ForbiddenOperation`] exists so a caller
//!    attempting one gets an explicit rejection instead of silence.
//! 3. **Evidence gate.** An action requires a measured effect with a
//!    confidence bucket at or above [`MIN_ACTION_CONFIDENCE`]. Insufficient
//!    evidence produces [`PolicyDecision::Hold`], never a guess.
//! 4. **Cooldown + rate limit.** Repeated decisions are idempotent and bounded
//!    by [`COOLDOWN_MS`] and [`MAX_ACTIONS_PER_HOUR`].
//! 5. **Rollback.** A regression, a failed verification, a bridge error or an
//!    explicit disable produces [`PolicyDecision::Rollback`].
//! 6. **No network.** The core policy is pure arithmetic over local inputs.

use serde::{Deserialize, Serialize};

use super::os_turbo_stats::{ConfidenceBucket, EffectDirection, MeasuredEffect};

/// Policy schema version. A mismatch forces a rollback rather than a silent
/// reinterpretation of persisted state.
pub const POLICY_VERSION: u32 = 1;

/// Minimum confidence required before the policy may act.
pub const MIN_ACTION_CONFIDENCE: ConfidenceBucket = ConfidenceBucket::Medium;

/// Minimum time between two applied actions.
pub const COOLDOWN_MS: u64 = 10 * 60 * 1000;

/// Maximum applied actions in any rolling hour.
pub const MAX_ACTIONS_PER_HOUR: u32 = 6;

/// Maximum consecutive regressions before the policy rolls back and stops.
pub const MAX_CONSECUTIVE_REGRESSIONS: u32 = 2;

/// The complete set of actions the policy may emit.
///
/// Deliberately tiny: OS Turbo only manages background-process scheduling and
/// the Windows power scheme. There is no variant for thermal, EC, firmware or
/// fan control, so such an action cannot be expressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    /// Apply EcoQoS execution-speed throttling to allowlisted background
    /// processes.
    ThrottleAllowlistedBackground,
    /// Restore allowlisted background processes to their captured state.
    RestoreAllowlistedBackground,
    /// Switch the active power scheme to the high-performance scheme.
    UseHighPerformanceScheme,
    /// Restore the power scheme captured before OS Turbo first acted.
    RestoreCapturedScheme,
}

impl PolicyAction {
    /// Every action the policy may emit, in deterministic order.
    pub const ALL: [PolicyAction; 4] = [
        PolicyAction::ThrottleAllowlistedBackground,
        PolicyAction::RestoreAllowlistedBackground,
        PolicyAction::UseHighPerformanceScheme,
        PolicyAction::RestoreCapturedScheme,
    ];

    /// `true` only for actions in [`PolicyAction::ALL`].
    pub fn is_allowed(self) -> bool {
        Self::ALL.contains(&self)
    }

    /// `true` when the action changes system state (as opposed to restoring it).
    pub fn is_mutating(self) -> bool {
        matches!(
            self,
            PolicyAction::ThrottleAllowlistedBackground | PolicyAction::UseHighPerformanceScheme
        )
    }
}

/// Operations the policy must never perform. Present so callers get an
/// explicit, testable rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenOperation {
    /// Writing CPU/package thermal limits (PL1/PL2).
    ThermalLimitWrite,
    /// Writing embedded-controller registers.
    EcRegisterWrite,
    /// Writing firmware or ACPI settings.
    FirmwareWrite,
    /// Writing fan curves or fan speed.
    FanControlWrite,
    /// Writing TDP setpoints.
    TdpWrite,
    /// Acting on a process outside the allowlist.
    UnallowlistedProcess,
    /// Executing a model-generated instruction directly.
    ModelGeneratedInstruction,
}

impl ForbiddenOperation {
    /// Human-readable reason, safe to log (no identifying data).
    pub fn reason(self) -> &'static str {
        match self {
            Self::ThermalLimitWrite => "thermal limit writes are out of scope for OS Turbo",
            Self::EcRegisterWrite => "EC register writes are out of scope for OS Turbo",
            Self::FirmwareWrite => "firmware writes are out of scope for OS Turbo",
            Self::FanControlWrite => "fan control is out of scope for OS Turbo",
            Self::TdpWrite => "TDP writes are out of scope for OS Turbo",
            Self::UnallowlistedProcess => "process is not on the OS Turbo allowlist",
            Self::ModelGeneratedInstruction => {
                "model-generated instructions are advisory and never executed"
            }
        }
    }
}

/// Why the policy declined to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    /// Evidence did not reach [`MIN_ACTION_CONFIDENCE`].
    InsufficientEvidence,
    /// The measured effect was neutral.
    NeutralEffect,
    /// A cooldown window is still open.
    CooldownActive,
    /// The hourly action budget is exhausted.
    RateLimited,
    /// OS Turbo is disabled by the user.
    Disabled,
    /// The persisted policy version does not match [`POLICY_VERSION`].
    PolicyVersionMismatch,
    /// The bridge reported an error, so no new action is attempted.
    BridgeError,
}

/// Why the policy rolled back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackReason {
    /// The measured effect was a regression.
    MeasuredRegression,
    /// Consecutive regressions reached [`MAX_CONSECUTIVE_REGRESSIONS`].
    RepeatedRegression,
    /// Post-action verification failed.
    VerificationFailed,
    /// The bridge reported an error.
    BridgeError,
    /// The user disabled OS Turbo.
    UserDisabled,
    /// The persisted policy version does not match.
    PolicyVersionMismatch,
}

/// The policy's decision for one evaluation cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Apply this allowlisted action.
    Apply { action: PolicyAction },
    /// Do nothing, with an explicit reason.
    Hold { reason: HoldReason },
    /// Undo the last applied action, with an explicit reason.
    Rollback { reason: RollbackReason },
}

/// Inputs to one policy evaluation. All values are local and non-identifying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyInputs {
    /// Persisted policy version.
    pub policy_version: u32,
    /// Whether the user has OS Turbo enabled.
    pub enabled: bool,
    /// Whether the bridge reported an error this cycle.
    pub bridge_error: bool,
    /// Whether post-action verification failed.
    pub verification_failed: bool,
    /// Measured effect for the current window, when one exists.
    pub effect: Option<MeasuredEffect>,
    /// Consecutive regressions observed so far.
    pub consecutive_regressions: u32,
    /// Milliseconds since the last applied action (`None` = never acted).
    pub ms_since_last_action: Option<u64>,
    /// Actions applied in the trailing hour.
    pub actions_last_hour: u32,
    /// Whether an action is currently applied and therefore reversible.
    pub action_applied: bool,
}

/// Evaluate the policy for one cycle.
///
/// Order of checks is fixed and documented so behaviour is reproducible:
/// version → disabled → bridge error → verification failure → repeated
/// regression → regression → cooldown → rate limit → evidence → neutral →
/// apply.
pub fn evaluate(inputs: &PolicyInputs) -> PolicyDecision {
    if inputs.policy_version != POLICY_VERSION {
        return PolicyDecision::Rollback {
            reason: RollbackReason::PolicyVersionMismatch,
        };
    }

    if !inputs.enabled {
        return if inputs.action_applied {
            PolicyDecision::Rollback {
                reason: RollbackReason::UserDisabled,
            }
        } else {
            PolicyDecision::Hold {
                reason: HoldReason::Disabled,
            }
        };
    }

    if inputs.bridge_error {
        return if inputs.action_applied {
            PolicyDecision::Rollback {
                reason: RollbackReason::BridgeError,
            }
        } else {
            PolicyDecision::Hold {
                reason: HoldReason::BridgeError,
            }
        };
    }

    if inputs.verification_failed {
        return PolicyDecision::Rollback {
            reason: RollbackReason::VerificationFailed,
        };
    }

    if inputs.consecutive_regressions >= MAX_CONSECUTIVE_REGRESSIONS {
        return PolicyDecision::Rollback {
            reason: RollbackReason::RepeatedRegression,
        };
    }

    let Some(effect) = inputs.effect.as_ref() else {
        return PolicyDecision::Hold {
            reason: HoldReason::InsufficientEvidence,
        };
    };

    if effect.direction == EffectDirection::Regression {
        return PolicyDecision::Rollback {
            reason: RollbackReason::MeasuredRegression,
        };
    }

    // A neutral measurement is reported as such rather than as missing
    // evidence: the comparison succeeded, it simply found no real change.
    if effect.direction == EffectDirection::Neutral {
        return PolicyDecision::Hold {
            reason: HoldReason::NeutralEffect,
        };
    }

    if let Some(elapsed) = inputs.ms_since_last_action {
        if elapsed < COOLDOWN_MS {
            return PolicyDecision::Hold {
                reason: HoldReason::CooldownActive,
            };
        }
    }

    if inputs.actions_last_hour >= MAX_ACTIONS_PER_HOUR {
        return PolicyDecision::Hold {
            reason: HoldReason::RateLimited,
        };
    }

    let confidence = super::os_turbo_stats::confidence_bucket(effect);
    if confidence < MIN_ACTION_CONFIDENCE {
        return PolicyDecision::Hold {
            reason: HoldReason::InsufficientEvidence,
        };
    }

    // Improvement with sufficient confidence: keep the throttling applied.
    // If nothing is applied yet, apply it; otherwise the state is already
    // correct and the decision is idempotent.
    if inputs.action_applied {
        PolicyDecision::Hold {
            reason: HoldReason::NeutralEffect,
        }
    } else {
        PolicyDecision::Apply {
            action: PolicyAction::ThrottleAllowlistedBackground,
        }
    }
}

/// Reject a forbidden operation with an explicit reason.
pub fn reject_forbidden(operation: ForbiddenOperation) -> Result<(), String> {
    Err(operation.reason().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::os_turbo_stats::{
        ConfidenceBucket, EffectDirection, MeasuredEffect, MetricKind, WorkloadClass,
    };

    fn effect(direction: EffectDirection, samples: u64, d: f64) -> MeasuredEffect {
        MeasuredEffect {
            metric: MetricKind::BackgroundCpuPercent,
            workload: WorkloadClass::Light,
            before_samples: samples,
            after_samples: samples,
            before_mean: 40.0,
            after_mean: 20.0,
            delta: -20.0,
            effect_size: Some(d),
            ci_low: -21.0,
            ci_high: -19.0,
            ci_excludes_zero: true,
            direction,
        }
    }

    fn base_inputs() -> PolicyInputs {
        PolicyInputs {
            policy_version: POLICY_VERSION,
            enabled: true,
            bridge_error: false,
            verification_failed: false,
            effect: Some(effect(EffectDirection::Improvement, 32, 4.0)),
            consecutive_regressions: 0,
            ms_since_last_action: Some(COOLDOWN_MS + 1),
            actions_last_hour: 0,
            action_applied: false,
        }
    }

    #[test]
    fn allowlist_contains_only_scheduler_and_power_actions() {
        for action in PolicyAction::ALL {
            assert!(action.is_allowed());
        }
        assert_eq!(PolicyAction::ALL.len(), 4);
        // Mutating actions are exactly the two that change state.
        let mutating: Vec<_> = PolicyAction::ALL
            .iter()
            .filter(|a| a.is_mutating())
            .copied()
            .collect();
        assert_eq!(
            mutating,
            vec![
                PolicyAction::ThrottleAllowlistedBackground,
                PolicyAction::UseHighPerformanceScheme
            ]
        );
    }

    #[test]
    fn forbidden_operations_are_rejected_with_reasons() {
        for operation in [
            ForbiddenOperation::ThermalLimitWrite,
            ForbiddenOperation::EcRegisterWrite,
            ForbiddenOperation::FirmwareWrite,
            ForbiddenOperation::FanControlWrite,
            ForbiddenOperation::TdpWrite,
            ForbiddenOperation::UnallowlistedProcess,
            ForbiddenOperation::ModelGeneratedInstruction,
        ] {
            let result = reject_forbidden(operation);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), operation.reason());
        }
    }

    #[test]
    fn policy_version_mismatch_rolls_back() {
        let mut inputs = base_inputs();
        inputs.policy_version = POLICY_VERSION + 1;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::PolicyVersionMismatch
            }
        );
    }

    #[test]
    fn disabled_without_applied_action_holds() {
        let mut inputs = base_inputs();
        inputs.enabled = false;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::Disabled
            }
        );
    }

    #[test]
    fn disabled_with_applied_action_rolls_back() {
        let mut inputs = base_inputs();
        inputs.enabled = false;
        inputs.action_applied = true;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::UserDisabled
            }
        );
    }

    #[test]
    fn bridge_error_rolls_back_when_action_applied() {
        let mut inputs = base_inputs();
        inputs.bridge_error = true;
        inputs.action_applied = true;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::BridgeError
            }
        );
    }

    #[test]
    fn verification_failure_rolls_back() {
        let mut inputs = base_inputs();
        inputs.verification_failed = true;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::VerificationFailed
            }
        );
    }

    #[test]
    fn repeated_regressions_roll_back() {
        let mut inputs = base_inputs();
        inputs.consecutive_regressions = MAX_CONSECUTIVE_REGRESSIONS;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::RepeatedRegression
            }
        );
    }

    #[test]
    fn measured_regression_rolls_back() {
        let mut inputs = base_inputs();
        inputs.effect = Some(effect(EffectDirection::Regression, 32, 4.0));
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::MeasuredRegression
            }
        );
    }

    #[test]
    fn missing_effect_holds_with_insufficient_evidence() {
        let mut inputs = base_inputs();
        inputs.effect = None;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::InsufficientEvidence
            }
        );
    }

    #[test]
    fn cooldown_blocks_a_second_action() {
        let mut inputs = base_inputs();
        inputs.ms_since_last_action = Some(COOLDOWN_MS - 1);
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::CooldownActive
            }
        );
    }

    #[test]
    fn rate_limit_blocks_after_budget_exhausted() {
        let mut inputs = base_inputs();
        inputs.actions_last_hour = MAX_ACTIONS_PER_HOUR;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::RateLimited
            }
        );
    }

    #[test]
    fn low_confidence_holds() {
        let mut inputs = base_inputs();
        // 8 samples clears the minimum but not twice the minimum → Low bucket.
        inputs.effect = Some(effect(EffectDirection::Improvement, 8, 4.0));
        assert_eq!(
            super::super::os_turbo_stats::confidence_bucket(inputs.effect.as_ref().unwrap()),
            ConfidenceBucket::Low
        );
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::InsufficientEvidence
            }
        );
    }

    #[test]
    fn neutral_effect_holds() {
        let mut inputs = base_inputs();
        inputs.effect = Some(effect(EffectDirection::Neutral, 32, 0.05));
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::NeutralEffect
            }
        );
    }

    #[test]
    fn sufficient_improvement_applies_the_allowlisted_action() {
        let inputs = base_inputs();
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Apply {
                action: PolicyAction::ThrottleAllowlistedBackground
            }
        );
    }

    #[test]
    fn repeated_decision_is_idempotent_when_already_applied() {
        let mut inputs = base_inputs();
        inputs.action_applied = true;
        // Already applied: no second mutation is emitted.
        assert!(matches!(evaluate(&inputs), PolicyDecision::Hold { .. }));
    }

    #[test]
    fn evaluation_order_is_deterministic() {
        // Version mismatch wins over every other condition.
        let mut inputs = base_inputs();
        inputs.policy_version = 99;
        inputs.enabled = false;
        inputs.bridge_error = true;
        inputs.verification_failed = true;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Rollback {
                reason: RollbackReason::PolicyVersionMismatch
            }
        );

        // Disabled wins over bridge error.
        let mut inputs = base_inputs();
        inputs.enabled = false;
        inputs.bridge_error = true;
        assert_eq!(
            evaluate(&inputs),
            PolicyDecision::Hold {
                reason: HoldReason::Disabled
            }
        );
    }

    #[test]
    fn decision_serialization_is_stable() {
        let decision = evaluate(&base_inputs());
        let first = serde_json::to_string(&decision).expect("json");
        let second = serde_json::to_string(&decision).expect("json");
        assert_eq!(first, second);
        assert!(first.contains("throttle_allowlisted_background"));
    }
}
