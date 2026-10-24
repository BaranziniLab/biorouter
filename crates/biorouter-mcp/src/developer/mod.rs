pub mod analyze;
mod background;
mod editor_models;
pub mod jail;
mod lang;
mod patch_content;
pub mod paths;
// Public so the stdio-extension spawn path in `biorouter` can share one
// definition of BioRouter's daemon-private environment (issue #57) instead of
// keeping a second copy that drifts.
pub mod shell;
pub(crate) mod text_editor;
pub(crate) mod undo_history;

pub mod rmcp_developer;

#[cfg(test)]
mod tests;
