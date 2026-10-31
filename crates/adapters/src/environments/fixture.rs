//! Shared cleanup for explicitly enabled local Docker integration fixtures only.
use super::{DockerEnvironmentRuntime, policy::verify_labels};
use crate::egress::EgressMetadata;
use bollard::{errors::Error, query_parameters::*};
use factory_core::environments::Environment;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) fn egress_metadata(installation: Uuid) -> EgressMetadata {
    let directory =
        std::env::var("FACTORY_ENV_TEST_DIRECTORY").expect("explicit prepared fixture directory");
    let mut metadata = EgressMetadata::load(directory).unwrap();
    metadata.installation_id = installation;
    metadata
}

pub(crate) fn synthetic_egress(installation: Uuid) -> EgressMetadata {
    EgressMetadata {
        schema_version: 1,
        installation_id: installation,
        image_id: format!("sha256:{}", "f".repeat(64)),
        host_ipv4: vec![std::net::Ipv4Addr::LOCALHOST],
    }
}

fn missing(error: &Error) -> bool {
    matches!(
        error,
        Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

pub(crate) async fn stop_proxy(runtime: &DockerEnvironmentRuntime) {
    let metadata = egress_metadata(runtime.metadata.installation_id());
    let proxy = runtime
        .docker
        .inspect_container(&metadata.proxy_name(), None::<InspectContainerOptions>)
        .await
        .unwrap();
    let value = serde_json::to_value(proxy).unwrap();
    let body = serde_json::to_value(metadata.proxy_body()).unwrap();
    assert_eq!(value["Config"]["Labels"], body["Labels"]);
    assert_eq!(value["Image"], metadata.image_id);
    runtime
        .docker
        .stop_container(
            value["Id"].as_str().unwrap(),
            Some(StopContainerOptions {
                t: Some(1),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
}

pub(crate) async fn cleanup(runtime: &DockerEnvironmentRuntime, environments: &[&Environment]) {
    for env in environments {
        match runtime
            .docker
            .inspect_container(&env.container_name, None::<InspectContainerOptions>)
            .await
        {
            Err(error) if missing(&error) => {}
            Err(error) => panic!("fixture container cleanup inspection failed: {error}"),
            Ok(container) => {
                let value = serde_json::to_value(container).unwrap();
                assert_eq!(value["Name"], format!("/{}", env.container_name));
                verify_labels(env, &value["Config"]["Labels"]).unwrap();
                remove_container(runtime, &value).await;
            }
        }
        match runtime.docker.inspect_volume(&env.volume_name).await {
            Err(error) if missing(&error) => {}
            Err(error) => panic!("fixture volume cleanup inspection failed: {error}"),
            Ok(volume) => {
                let value = serde_json::to_value(volume).unwrap();
                assert_eq!(value["Name"], env.volume_name);
                assert_eq!(value["Driver"], "local");
                verify_labels(env, &value["Labels"]).unwrap();
                runtime
                    .docker
                    .remove_volume(&env.volume_name, None::<RemoveVolumeOptions>)
                    .await
                    .unwrap();
            }
        }
    }
    cleanup_networks(runtime, environments).await;
}

async fn remove_container(runtime: &DockerEnvironmentRuntime, value: &Value) {
    let id = value["Id"].as_str().unwrap();
    assert_eq!(id.len(), 64);
    assert!(id.bytes().all(|c| c.is_ascii_hexdigit()));
    if value["State"]["Running"] == true {
        runtime
            .docker
            .stop_container(
                id,
                Some(StopContainerOptions {
                    t: Some(1),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
    }
    runtime
        .docker
        .remove_container(id, None::<RemoveContainerOptions>)
        .await
        .unwrap();
}

pub(crate) async fn cleanup_networks(
    runtime: &DockerEnvironmentRuntime,
    environments: &[&Environment],
) {
    let metadata = egress_metadata(runtime.metadata.installation_id());
    match runtime
        .docker
        .inspect_container(&metadata.proxy_name(), None::<InspectContainerOptions>)
        .await
    {
        Err(error) if missing(&error) => {}
        Err(error) => panic!("fixture proxy cleanup inspection failed: {error}"),
        Ok(proxy) => {
            let value = serde_json::to_value(proxy).unwrap();
            let body = serde_json::to_value(metadata.proxy_body()).unwrap();
            assert_eq!(value["Config"]["Labels"], body["Labels"]);
            assert_eq!(value["Name"], format!("/{}", metadata.proxy_name()));
            assert_eq!(value["Image"], metadata.image_id);
            assert!(value["Mounts"].as_array().unwrap().is_empty());
            remove_container(runtime, &value).await;
        }
    }
    let targets = environments.iter().map(|env| Some(env.id)).chain([None]);
    for environment in targets {
        let body = serde_json::to_value(metadata.network_body(environment)).unwrap();
        let name = body["Name"].as_str().unwrap();
        match runtime
            .docker
            .inspect_network(name, None::<InspectNetworkOptions>)
            .await
        {
            Err(error) if missing(&error) => {}
            Err(error) => panic!("fixture network cleanup inspection failed: {error}"),
            Ok(network) => {
                let value = serde_json::to_value(network).unwrap();
                for field in ["Name", "Labels", "Driver", "Internal", "Options"] {
                    assert_eq!(value[field], body[field]);
                }
                assert!(value["Containers"] == json!({}) || value["Containers"].is_null());
                let id = value["Id"].as_str().unwrap();
                assert_eq!(id.len(), 64);
                runtime.docker.remove_network(id).await.unwrap();
            }
        }
    }
}
