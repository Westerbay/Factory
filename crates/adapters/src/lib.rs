//! Concrete provider adapters; credentials are injected by the runtime composition.

pub mod attachments;
pub mod claude_account;
pub mod claude_execution;
pub mod codex_account;
pub mod codex_execution;
pub mod egress;
pub mod environments;
mod execution_diagnostics;
pub mod feature_codec;
pub mod identity;
mod json_stream;
pub mod projects;
pub mod terminals;
