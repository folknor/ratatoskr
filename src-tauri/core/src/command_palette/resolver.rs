use super::context::CommandContext;
use super::id::CommandId;
use super::input::OptionItem;

/// Resolves dynamic input for parameterized commands.
///
/// Same pattern as `ProgressReporter`: core defines the trait, the app layer
/// provides a concrete implementation. The registry is immutable static data;
/// the resolver needs DB access and live account state.
///
/// - Tauri app: `TauriInputResolver` queries `DbState` for folders, labels, etc.
/// - Future iced app: queries its own model.
pub trait CommandInputResolver: Send + Sync {
    /// Return available options for a `ListPicker` parameter step.
    ///
    /// Only called for `ParamDef::ListPicker` steps. `DateTime`, `Text`, and
    /// `Enum` steps are handled by the frontend directly (date picker, text
    /// input, static enum list from the schema). The resolver is not involved
    /// in those input flows.
    ///
    /// `prior_selections` contains the values chosen in steps `0..param_index`
    /// for `Sequence` schemas. Empty for `Single` schemas. Enables
    /// context-dependent options (e.g., step 2 options filtered by step 1's
    /// choice).
    fn get_options(
        &self,
        command_id: CommandId,
        param_index: usize,
        prior_selections: &[String],
        ctx: &CommandContext,
    ) -> Result<Vec<OptionItem>, String>;

    /// Validate a selected value for a parameter step.
    ///
    /// Called for any step type:
    /// - `ListPicker`: value is the selected `OptionItem.id`
    /// - `DateTime`: value is a stringified unix timestamp
    /// - `Enum`: value is the `EnumOption.value`
    /// - `Text`: value is the user's input string
    ///
    /// `prior_selections` contains values chosen in steps `0..param_index`.
    /// Enables cross-field validation (e.g., "folder cannot equal current
    /// folder after prior step selection").
    fn validate_option(
        &self,
        command_id: CommandId,
        param_index: usize,
        value: &str,
        prior_selections: &[String],
        ctx: &CommandContext,
    ) -> Result<(), String>;
}
