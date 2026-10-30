//! Bounded, local-only Docker lifecycle. No pull, build, removal, or ambient daemon settings.
mod policy;
mod preparation;
pub(crate) mod process;
mod runtime;

pub use policy::IsolationPolicy;
pub use preparation::{InstallationGuard, POLICY_VERSION, PreparationMetadata};
pub use runtime::DockerEnvironmentRuntime;

#[cfg(test)]
mod tests;
