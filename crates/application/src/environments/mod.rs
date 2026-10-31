//! Persisted user intent and per-environment reconciliation through neutral ports.
mod reconcile;
mod service;

pub use reconcile::EnvironmentReconciler;
pub use service::{EnvironmentAvailability, EnvironmentService};

#[cfg(test)]
mod tests;
