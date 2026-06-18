use bifrost_types::{AccountError, RecoveryClass};
use service_api::actions::{ActionError, RemoteFailureKind};
use service_api::{OperationResult, RemoteFailure};

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

fn recovery_to_failure_kind(recovery: &RecoveryClass) -> RemoteFailureKind {
    match recovery {
        RecoveryClass::Retry(_) | RecoveryClass::Reconcile(_) | RecoveryClass::Engine(_) => {
            RemoteFailureKind::Transient
        }
        RecoveryClass::Unsupported(_) => RemoteFailureKind::NotImplemented,
        RecoveryClass::AuthLost
        | RecoveryClass::NeedsAdminConsent { .. }
        | RecoveryClass::NeedsPolicyChange
        | RecoveryClass::NoPermission { .. }
        | RecoveryClass::ClientBug
        | RecoveryClass::ProviderContractViolation
        | RecoveryClass::ProviderRefused
        | RecoveryClass::UnknownPermanent => RemoteFailureKind::Permanent,
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
            (engine_error(), RemoteFailureKind::Transient),
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
        for err in [retry_error(), reconcile_error(), engine_error()] {
            assert!(
                account_error_to_action_error(&err).is_retryable(),
                "expected retryable for {:?}",
                err.recovery(),
            );
        }
        for err in [
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

    fn engine_error() -> bifrost_types::AccountError {
        AccountErrorBuilder::new(
            AccountErrorKind::SyncState(SyncStateErrorKind::SchemaIncompatible),
            Cause::State(StateCause::SchemaIncompatible),
        )
        .try_build()
        .expect("valid engine error")
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
