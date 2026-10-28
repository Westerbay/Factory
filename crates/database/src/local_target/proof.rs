use super::TargetError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Untrusted compatibility shape shared with the existing TypeScript verifier.
/// Passing this check alone does not produce a connection capability.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetProof {
    pub project: String,
    pub owner: String,
    pub directory: String,
    pub database: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: u32,
    pub container: String,
    pub labels: BTreeMap<String, String>,
    pub published_host: String,
    pub published_port: u32,
}

pub(super) fn fixture_project(project: &str) -> bool {
    project.strip_prefix("factory-test-").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

pub(super) fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, c)| {
            if [8, 13, 18, 23].contains(&index) {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
            }
        })
}

fn absolute_directory(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with('/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && [b'/', b'\\'].contains(&bytes[2]))
}

pub(super) fn matching_labels(proof: &TargetProof, labels: &BTreeMap<String, String>) -> bool {
    labels.get("com.docker.compose.project") == Some(&proof.project)
        && labels.get("io.factory.owner") == Some(&proof.owner)
        && labels.get("io.factory.directory") == Some(&proof.directory)
}

pub fn check_proof(proof: &TargetProof) -> Result<(), TargetError> {
    let database = if proof.project == "factory-local" {
        "factory_local"
    } else if fixture_project(&proof.project) {
        "factory_test"
    } else {
        return Err(TargetError::InvalidProof);
    };
    if !uuid(&proof.owner)
        || !absolute_directory(&proof.directory)
        || proof.container != format!("{}-postgres-1", proof.project)
        || !matching_labels(proof, &proof.labels)
        || proof.host != "127.0.0.1"
        || proof.published_host != proof.host
        || proof.published_port != proof.port
        || !(1025..=65535).contains(&proof.port)
        || proof.database != database
        || proof.username != "factory"
        || proof.password != "local-fixture-only"
    {
        return Err(TargetError::InvalidProof);
    }
    Ok(())
}
