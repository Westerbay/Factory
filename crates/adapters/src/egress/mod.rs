//! Installation-owned egress resources. Never attach an existing user container here.
mod policy;
mod runtime;

pub use policy::{EgressMetadata, UserNetwork};
pub use runtime::DockerEgress;

#[cfg(test)]
mod integration;
#[cfg(test)]
mod tests;
