//! Persisted user intent and per-environment reconciliation through neutral ports.
mod reconcile;
mod service;
mod upgrade;

pub use reconcile::EnvironmentReconciler;
pub use service::{EnvironmentAvailability, EnvironmentService};
pub use upgrade::{EnvironmentUpgradePreview, EnvironmentUpgradeService};

#[cfg(test)]
mod tests;
