//! Installation-owned egress resources. Never attach an existing user container here.
mod policy;
mod runtime;

pub use policy::{EgressMetadata, PROXY_URL, UserNetwork, user_network_name};
pub use runtime::DockerEgress;

#[cfg(test)]
mod integration;
#[cfg(test)]
mod tests;
