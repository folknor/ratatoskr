use bifrost_types::{AccountError, EngineDirective, RecoveryClass};
use service_api::actions::{ActionError, RemoteFailureKind};
use service_api::{OperationResult, RemoteFailure, SyncPauseReason};

pub fn account_error_to_action_error(err: &AccountError) -> ActionError {
    let kind = recovery_to_failure_kind(err.recovery());
    ActionError::remote_with_kind(kind, err.message_key())
}

pub fn account_error_to_operation_result(err: &AccountError) -> OperationResult {
    let action_error = account_error_to_action_error(err);
    let ActionError::Remote { kind, message } = action_error else {
        unreachable!("account_error_to_action_error only produces remote failures")
    };
    OperationResult::RemoteFailure {
        failure: RemoteFailure {
            provider_message: message,
            http_status: None,
            retryable: matches!(
                kind,
                RemoteFailureKind::Transient | RemoteFailureKind::Unknown
            ),
        },
    }
}

/// Map a latched terminal `OperationResult` to the wire pause reason
/// surfaced to the UI (B3c § 4.1, review finding F3: derive the banner from
/// the terminal error, not from the bare `PauseReason`, which carries no
/// recovery class). Lives here, beside the `AccountError -> OperationResult`
/// map, so the auth-loss heuristic sits next to the namespace it depends on.
///
/// The firewall forbids `RecoveryClass` from crossing into `service-api`, so
/// the auth-loss signal is recovered from the stable `message_key` namespace
/// instead: the only non-retryable `auth.*` keys bifrost derives are the
/// three `RecoveryClass::AuthLost` kinds (`auth.expired`, `auth.revoked`,
/// `auth.reauthorization-required`). `auth.refresh-transient` is also
/// `auth.`-prefixed but is retryable, so the `!retryable` guard excludes it.
/// Everything else maps to the generic attention-required banner. A bifrost
/// bump that renames those keys is caught by the unit test below.
#[must_use]
pub fn pause_reason_to_wire(result: &OperationResult) -> SyncPauseReason {
    if let OperationResult::RemoteFailure { failure } = result
        && !failure.retryable
        && failure.provider_message.starts_with("auth.")
    {
        return SyncPauseReason::NeedsReauth;
    }
    SyncPauseReason::NeedsAttention
}

fn recovery_to_failure_kind(recovery: &RecoveryClass) -> RemoteFailureKind {
    match recovery {
        RecoveryClass::Retry(_) | RecoveryClass::Reconcile(_) => RemoteFailureKind::Transient,
        // Engine-class failures split by whether the engine can self-heal.
        // Auto-recoverable directives (scope/account restart, strategy or
        // capability downgrade, scope disable) leave a later pending-ops
        // re-drive a path to success once the engine resumes -> retryable.
        // SchemaIncompatible / OperatorOverrideRequired cannot clear without
        // a migration or operator action, so an action retry can NEVER
        // succeed -> terminal (the AuthLost analogue).
        RecoveryClass::Engine(directive) => match directive {
            EngineDirective::SchemaIncompatible
            | EngineDirective::OperatorOverrideRequired { .. } => RemoteFailureKind::Permanent,
            EngineDirective::RestartScope(_)
            | EngineDirective::RestartAccount
            | EngineDirective::DowngradeStrategy(_)
            | EngineDirective::DowngradeCapabilityForScope(_)
            | EngineDirective::DisableScope(_) => RemoteFailureKind::Transient,
            // `EngineDirective` is `#[non_exhaustive]`: a new upstream
            // directive defaults to the conservative auto-recoverable
            // (retryable) classification - engine directives are recovery
            // work, and the bounded per-op retry budget caps any wasted
            // re-drive. Re-audited on every bifrost bump.
            _ => RemoteFailureKind::Transient,
        },
        RecoveryClass::Unsupported(_) => RemoteFailureKind::NotImplemented,
        RecoveryClass::AuthLost
        | RecoveryClass::NeedsAdminConsent { .. }
        | RecoveryClass::NeedsPolicyChange
        | RecoveryClass::NoPermission { .. }
        | RecoveryClass::ClientBug
        | RecoveryClass::ProviderContractViolation
        | RecoveryClass::ProviderRefused
        | RecoveryClass::UnknownPermanent => RemoteFailureKind::Permanent,
        // `RecoveryClass` is `#[non_exhaustive]` upstream, so this wildcard is
        // REQUIRED to compile. It is the documented conservative-terminal
        // default: a new upstream class is treated as terminal (never enqueues
        // a doomed retry) until a bifrost-bump revalidation names it above. Do
        // NOT delete this arm.
        _ => RemoteFailureKind::Permanent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_types::{
        AccessCause, AccessErrorKind, AccountErrorBuilder, AccountErrorKind, AccountOperation,
        AttemptCause, AuthCause, AuthErrorKind, Cause, DiagnosticText, Protocol, ProtocolErrorKind,
        RequestCause, RequestErrorKind, ResourceKind, ServerCause, ServerErrorKind, StateCause,
        SyncStateErrorKind, TransmissionState, WireCause,
    };

    #[test]
    fn bifrost_error_map_covers_every_recovery_class() {
        // Coverage mechanism: the per-bump revalidation rule (the spec's
        // explicit fallback, B1 3.3 totality note). Tying this test directly
        // to bifrost's `every_recovery_variant` catalog is not feasible at the
        // frozen surface: that catalog is a non-`pub` `fn` inside a
        // `#[cfg(test)] mod` in bifrost-types `crates/types/src/error/recovery.rs`,
        // so it does not cross the crate boundary. Enforcement is therefore the
        // documented checklist: every `../bifrost` commit bump re-reads that
        // `recovery.rs` variant list and re-audits the 3.3 table, updating the
        // explicit known-variant cases below so a new upstream variant cannot
        // slip silently through the mapping's `_` arm. This is weaker than a
        // catalog-tied assertion but is the strongest mechanism the frozen
        // surface permits.
        let cases = [
            (retry_error(), RemoteFailureKind::Transient),
            (reconcile_error(), RemoteFailureKind::Transient),
            (restart_account_error(), RemoteFailureKind::Transient),
            (schema_incompatible_error(), RemoteFailureKind::Permanent),
            (operator_override_error(), RemoteFailureKind::Permanent),
            (auth_lost_error(), RemoteFailureKind::Permanent),
            (admin_consent_error(), RemoteFailureKind::Permanent),
            (policy_error(), RemoteFailureKind::Permanent),
            (no_permission_error(), RemoteFailureKind::Permanent),
            (unsupported_error(), RemoteFailureKind::NotImplemented),
            (client_bug_error(), RemoteFailureKind::Permanent),
            (provider_contract_error(), RemoteFailureKind::Permanent),
            (provider_refused_error(), RemoteFailureKind::Permanent),
            (unknown_permanent_error(), RemoteFailureKind::Permanent),
        ];

        for (err, expected) in cases {
            let ActionError::Remote { kind, .. } = account_error_to_action_error(&err) else {
                panic!("account errors map only to remote action errors");
            };
            assert_eq!(kind, expected, "recovery was {:?}", err.recovery());
        }
    }

    #[test]
    fn bifrost_error_map_retryable_classes_are_transient() {
        for err in [retry_error(), reconcile_error(), restart_account_error()] {
            assert!(
                account_error_to_action_error(&err).is_retryable(),
                "expected retryable for {:?}",
                err.recovery(),
            );
        }
        for err in [
            schema_incompatible_error(),
            operator_override_error(),
            auth_lost_error(),
            admin_consent_error(),
            policy_error(),
            no_permission_error(),
            unsupported_error(),
            client_bug_error(),
            provider_contract_error(),
            provider_refused_error(),
            unknown_permanent_error(),
        ] {
            assert!(
                !account_error_to_action_error(&err).is_retryable(),
                "expected terminal for {:?}",
                err.recovery(),
            );
        }
    }

    #[test]
    fn bifrost_error_map_message_is_message_key_not_raw() {
        let err = AccountErrorBuilder::new(
            AccountErrorKind::Request(RequestErrorKind::Malformed),
            Cause::Request(RequestCause::Malformed {
                detail: DiagnosticText::support_only("secret raw provider text"),
            }),
        )
        .try_build()
        .expect("valid account error");

        let OperationResult::RemoteFailure { failure } = account_error_to_operation_result(&err)
        else {
            panic!("expected remote failure");
        };

        assert_eq!(failure.provider_message, err.message_key());
        assert!(!failure.provider_message.contains("secret"));
    }

    #[test]
    fn pause_reason_to_wire_maps_auth_loss_to_reauth() {
        // The three RecoveryClass::AuthLost kinds carry non-retryable
        // `auth.*` message keys and must surface NeedsReauth.
        for err in [
            auth_lost_error(),
            AccountErrorBuilder::new(
                AccountErrorKind::Authentication(AuthErrorKind::Expired),
                Cause::Auth(AuthCause::Expired),
            )
            .try_build()
            .expect("valid auth-expired error"),
            AccountErrorBuilder::new(
                AccountErrorKind::Authentication(AuthErrorKind::ReauthorizationRequired),
                Cause::Auth(AuthCause::ReauthorizationRequired),
            )
            .try_build()
            .expect("valid reauth-required error"),
        ] {
            let result = account_error_to_operation_result(&err);
            assert_eq!(
                pause_reason_to_wire(&result),
                SyncPauseReason::NeedsReauth,
                "message key was {:?}",
                err.message_key(),
            );
        }
    }

    #[test]
    fn pause_reason_to_wire_maps_non_auth_and_retryable_to_attention() {
        // A retryable auth refresh and every non-auth terminal map to the
        // generic attention banner, never NeedsReauth.
        for err in [
            retry_error(),
            unknown_permanent_error(),
            no_permission_error(),
            policy_error(),
        ] {
            let result = account_error_to_operation_result(&err);
            assert_eq!(
                pause_reason_to_wire(&result),
                SyncPauseReason::NeedsAttention,
                "message key was {:?}",
                err.message_key(),
            );
        }
    }

    fn retry_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Server(ServerErrorKind::Unavailable),
            Cause::Server(ServerCause::Unavailable { retry_hint: None }),
        )
        .push_cause(Cause::Attempt(AttemptCause::new(TransmissionState::Unsent)))
        .operation(AccountOperation::Discover)
        .try_build()
        .expect("valid retry error")
    }

    fn reconcile_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Server(ServerErrorKind::Unavailable),
            Cause::Server(ServerCause::Unavailable { retry_hint: None }),
        )
        .push_cause(Cause::Attempt(AttemptCause::new(
            TransmissionState::InFlight,
        )))
        .operation(AccountOperation::Send)
        .try_build()
        .expect("valid reconcile error")
    }

    fn schema_incompatible_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::SyncState(SyncStateErrorKind::SchemaIncompatible),
            Cause::State(StateCause::SchemaIncompatible),
        )
        .try_build()
        .expect("valid schema-incompatible error")
    }

    fn restart_account_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::SyncState(SyncStateErrorKind::CapabilityChanged),
            Cause::State(StateCause::CapabilityChanged { delta: None }),
        )
        .try_build()
        .expect("valid restart-account error")
    }

    fn operator_override_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::SyncState(SyncStateErrorKind::OperatorOverrideNeeded),
            Cause::State(StateCause::OperatorOverrideNeeded {
                reason: "operator requested pause".to_string(),
            }),
        )
        .try_build()
        .expect("valid operator-override error")
    }

    fn auth_lost_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Authentication(AuthErrorKind::Revoked),
            Cause::Auth(AuthCause::Revoked),
        )
        .try_build()
        .expect("valid auth-lost error")
    }

    fn admin_consent_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Authorization(AccessErrorKind::AdminConsentRequired),
            Cause::Access(AccessCause::AdminConsentRequired { needed: "scope.x" }),
        )
        .try_build()
        .expect("valid admin-consent error")
    }

    fn policy_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Authorization(AccessErrorKind::PolicyBlocked),
            Cause::Access(AccessCause::PolicyBlocked),
        )
        .try_build()
        .expect("valid policy error")
    }

    fn no_permission_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Authorization(AccessErrorKind::PermissionDenied),
            Cause::Access(AccessCause::PermissionDenied { resource: None }),
        )
        .try_build()
        .expect("valid permission error")
    }

    fn unsupported_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Unsupported(AccountOperation::Send),
            Cause::Request(RequestCause::Unsupported {
                operation: AccountOperation::Send,
            }),
        )
        .try_build()
        .expect("valid unsupported error")
    }

    fn client_bug_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Request(RequestErrorKind::Malformed),
            Cause::Request(RequestCause::Malformed {
                detail: DiagnosticText::support_only("malformed"),
            }),
        )
        .try_build()
        .expect("valid client-bug error")
    }

    fn provider_contract_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Protocol(ProtocolErrorKind::MissingField),
            Cause::Wire(WireCause::MalformedResponse {
                protocol: Protocol::Jmap,
                detail: None,
            }),
        )
        .try_build()
        .expect("valid provider-contract error")
    }

    fn provider_refused_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::NotFound(ResourceKind::Message),
            Cause::Request(RequestCause::NotFound {
                what: ResourceKind::Message,
                id: Some("message-1".to_string()),
            }),
        )
        .try_build()
        .expect("valid provider-refused error")
    }

    fn unknown_permanent_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::Protocol(ProtocolErrorKind::Unknown),
            Cause::Wire(WireCause::MalformedResponse {
                protocol: Protocol::Jmap,
                detail: None,
            }),
        )
        .try_build()
        .expect("valid unknown-permanent error")
    }
}
