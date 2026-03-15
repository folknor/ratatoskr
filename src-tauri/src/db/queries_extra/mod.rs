// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

/// Generates a `#[tauri::command]` async wrapper that forwards to the
/// identically-named function in `ratatoskr_core::db::queries_extra`.
///
/// # Syntax
///
/// ```ignore
/// db_command! {
///     fn db_example(state, param1: String, param2: Option<i64>) -> Vec<Foo>;
/// }
/// ```
///
/// The first parameter is always `state: State<'_, DbState>` (written just as
/// `state` in the macro invocation).  It is passed as `&state` to the core
/// function.  All remaining parameters are forwarded by value.
macro_rules! db_command {
    // Base case: single function
    (
        fn $name:ident(state $(, $p:ident : $t:ty)* $(,)?) -> $ret:ty;
    ) => {
        #[tauri::command]
        pub async fn $name(
            state: tauri::State<'_, crate::db::DbState>,
            $($p: $t,)*
        ) -> Result<$ret, String> {
            ratatoskr_core::db::queries_extra::$name(&state, $($p,)*).await
        }
    };

    // Multiple functions
    (
        $(
            fn $name:ident(state $(, $p:ident : $t:ty)* $(,)?) -> $ret:ty;
        )+
    ) => {
        $(
            db_command! {
                fn $name(state $(, $p: $t)*) -> $ret;
            }
        )+
    };
}

mod contacts;
mod filters;
mod accounts;
mod threads;
mod tasks;
mod calendar;
mod bundles;
mod email_ops;
mod cache;

pub use contacts::*;
pub use filters::*;
pub use accounts::*;
pub use threads::*;
pub use tasks::*;
pub use calendar::*;
pub use bundles::*;
pub use email_ops::*;
pub use cache::*;
