//! Bounded, local-only Docker lifecycle. No pull, build, removal, or ambient daemon settings.
pub(crate) mod feature_execution;
mod policy;
mod preparation;
pub(crate) mod process;
mod runtime;

pub use feature_execution::DockerFeatureExecutionRuntime;
pub use policy::IsolationPolicy;
pub use preparation::{InstallationGuard, POLICY_VERSION, PreparationMetadata};
pub use runtime::DockerEnvironmentRuntime;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;
