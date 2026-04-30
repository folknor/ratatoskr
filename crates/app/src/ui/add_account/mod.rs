//! Add Account wizard - multi-step state machine and views.
//!
//! Phases 2-3 of the accounts implementation spec. The wizard handles
//! first-launch onboarding and subsequent account additions.

mod discovery;
mod email_input;
mod identity;
mod manual_config;
mod oauth;
mod password_auth;
mod state;
mod views;

pub use state::{AddAccountEvent, AddAccountMessage, AddAccountWizard};
