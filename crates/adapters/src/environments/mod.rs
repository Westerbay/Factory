//! Bounded, local-only Docker lifecycle. No pull, build, removal, or ambient daemon settings.
mod policy;
mod preparation;
pub(crate) mod process;
mod runtime;
mod upgrade;

pub use policy::IsolationPolicy;
pub use preparation::{InstallationGuard, POLICY_VERSION, PreparationMetadata};
pub use runtime::DockerEnvironmentRuntime;
pub use upgrade::DockerEnvironmentUpgradeRuntime;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;
