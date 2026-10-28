use super::{TargetError, TargetProof, proof::matching_labels};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Binding {
    host_ip: String,
    host_port: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HostConfig {
    port_bindings: BTreeMap<String, Option<Vec<Binding>>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Network {
    ports: BTreeMap<String, Option<Vec<Binding>>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Config {
    env: Vec<String>,
    labels: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Mount {
    destination: String,
    #[serde(rename = "Type")]
    kind: String,
    name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct State {
    running: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Container {
    name: String,
    state: State,
    config: Config,
    host_config: HostConfig,
    network_settings: Network,
    mounts: Vec<Mount>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Volume {
    name: String,
    labels: BTreeMap<String, String>,
}

fn binding_matches(bindings: &BTreeMap<String, Option<Vec<Binding>>>, proof: &TargetProof) -> bool {
    bindings
        .get("5432/tcp")
        .and_then(Option::as_ref)
        .is_some_and(|values| {
            values.len() == 1
                && values[0].host_ip == proof.host
                && values[0].host_port.parse::<u32>().ok() == Some(proof.port)
        })
}

pub(super) fn verify(
    proof: &TargetProof,
    container_json: &[u8],
    volume_json: &[u8],
) -> Result<(), TargetError> {
    let containers: Vec<Container> =
        serde_json::from_slice(container_json).map_err(|_| TargetError::InspectionMismatch)?;
    let volumes: Vec<Volume> =
        serde_json::from_slice(volume_json).map_err(|_| TargetError::InspectionMismatch)?;
    let [container] = containers.as_slice() else {
        return Err(TargetError::InspectionMismatch);
    };
    let [volume] = volumes.as_slice() else {
        return Err(TargetError::InspectionMismatch);
    };
    let volume_name = format!("{}_postgres_data", proof.project);
    let mounts: Vec<_> = container
        .mounts
        .iter()
        .filter(|mount| mount.destination == "/var/lib/postgresql/data")
        .collect();
    let environment_matches = [
        ("POSTGRES_DB", &proof.database),
        ("POSTGRES_USER", &proof.username),
        ("POSTGRES_PASSWORD", &proof.password),
    ]
    .iter()
    .all(|(name, value)| {
        let prefix = format!("{name}=");
        let entries: Vec<_> = container
            .config
            .env
            .iter()
            .filter(|entry| entry.starts_with(&prefix))
            .collect();
        entries.len() == 1 && *entries[0] == format!("{name}={value}")
    });
    if container.name != format!("/{}", proof.container)
        || !container.state.running
        || !matching_labels(proof, &container.config.labels)
        || !matching_labels(proof, &volume.labels)
        || !binding_matches(&container.host_config.port_bindings, proof)
        || !binding_matches(&container.network_settings.ports, proof)
        || !environment_matches
        || volume.name != volume_name
        || mounts.len() != 1
        || mounts[0].kind != "volume"
        || mounts[0].name.as_deref() != Some(volume_name.as_str())
    {
        return Err(TargetError::InspectionMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> (TargetProof, Value, Value) {
        let proof = super::super::expected(
            "/fixture".into(),
            "factory-test-abc",
            "b52c14b8-a06e-48ee-ab1d-2194cda60568",
            54321,
        );
        let bindings = json!({"5432/tcp": [{"HostIp":"127.0.0.1","HostPort":"54321"}]});
        let volume = json!([{"Name":"factory-test-abc_postgres_data","Labels":proof.labels}]);
        let container = json!([{
            "Name":"/factory-test-abc-postgres-1", "State":{"Running":true},
            "Config":{"Labels":proof.labels,"Env":["POSTGRES_DB=factory_test","POSTGRES_USER=factory","POSTGRES_PASSWORD=local-fixture-only"]},
            "HostConfig":{"PortBindings":bindings}, "NetworkSettings":{"Ports":bindings},
            "Mounts":[{"Destination":"/var/lib/postgresql/data","Type":"volume","Name":"factory-test-abc_postgres_data"}]
        }]);
        (proof, container, volume)
    }
    fn accepts(proof: &TargetProof, container: &Value, volume: &Value) -> bool {
        verify(
            proof,
            &serde_json::to_vec(container).unwrap(),
            &serde_json::to_vec(volume).unwrap(),
        )
        .is_ok()
    }

    #[test]
    fn live_proof_requires_both_owned_resources_and_exact_configuration() {
        let (proof, container, volume) = fixture();
        assert!(accepts(&proof, &container, &volume));
        for (pointer, value) in [
            ("/0/Name", json!("/other")),
            ("/0/State/Running", json!(false)),
            ("/0/Config/Labels/io.factory.owner", json!("other")),
            ("/0/Config/Env", json!(["POSTGRES_DB=other"])),
            (
                "/0/HostConfig/PortBindings/5432~1tcp/0/HostIp",
                json!("0.0.0.0"),
            ),
            (
                "/0/HostConfig/PortBindings/5432~1tcp/0/HostPort",
                json!("54322"),
            ),
            ("/0/NetworkSettings/Ports/5432~1tcp", json!([])),
            ("/0/NetworkSettings/Ports/5432~1tcp/0/HostIp", json!("::")),
            ("/0/Mounts/0/Type", json!("bind")),
            ("/0/Mounts/0/Name", json!("unowned")),
        ] {
            let mut changed = container.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(!accepts(&proof, &changed, &volume), "{pointer}");
        }
        for pointer in [
            "/0/Name",
            "/0/Labels/io.factory.owner",
            "/0/Labels/io.factory.directory",
            "/0/Labels/com.docker.compose.project",
        ] {
            let mut changed = volume.clone();
            *changed.pointer_mut(pointer).unwrap() = json!("other");
            assert!(!accepts(&proof, &container, &changed), "{pointer}");
        }
    }

    #[test]
    fn ambiguous_or_malformed_inspection_is_rejected() {
        let (proof, container, volume) = fixture();
        assert!(!accepts(&proof, &json!([]), &volume));
        assert!(!accepts(
            &proof,
            &json!([container[0], container[0]]),
            &volume
        ));
        assert!(!accepts(&proof, &container, &json!([])));
        let mut changed = container.clone();
        changed[0]["Config"]["Env"]
            .as_array_mut()
            .unwrap()
            .push(json!("POSTGRES_DB=other"));
        assert!(!accepts(&proof, &changed, &volume));
        let mut changed = container.clone();
        changed[0]["Mounts"]
            .as_array_mut()
            .unwrap()
            .push(container[0]["Mounts"][0].clone());
        assert!(!accepts(&proof, &changed, &volume));
        assert!(verify(&proof, b"not-json", b"[]").is_err());
    }
}
