use super::policy::*;
use factory_core::environments::EnvironmentError;
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use uuid::Uuid;

fn metadata() -> EgressMetadata {
    EgressMetadata {
        schema_version: 1,
        installation_id: Uuid::new_v4(),
        image_id: format!("sha256:{}", "a".repeat(64)),
        host_ipv4: vec![Ipv4Addr::LOCALHOST],
    }
}

fn network(metadata: &EgressMetadata, environment: Option<Uuid>) -> Value {
    let mut actual = serde_json::to_value(metadata.network_body(environment)).unwrap();
    actual["Id"] = json!("b".repeat(64));
    actual["Scope"] = json!("local");
    actual["IPAM"] = json!({"Driver":"default"});
    actual
}

#[test]
fn user_network_refuses_nat_ipv6_and_foreign_ownership() {
    let metadata = metadata();
    let environment = Uuid::new_v4();
    let valid = network(&metadata, Some(environment));
    assert!(metadata.verify_network(Some(environment), &valid).is_ok());
    for (path, replacement, expected) in [
        ("/Internal", json!(false), EnvironmentError::PolicyMismatch),
        ("/EnableIPv6", json!(true), EnvironmentError::PolicyMismatch),
        (
            "/Options/com.docker.network.bridge.gateway_mode_ipv4",
            json!("nat"),
            EnvironmentError::PolicyMismatch,
        ),
        (
            "/Labels/io.factory.environment",
            json!(Uuid::new_v4().to_string()),
            EnvironmentError::OwnershipConflict,
        ),
        ("/Driver", json!("host"), EnvironmentError::PolicyMismatch),
    ] {
        let mut altered = valid.clone();
        *altered.pointer_mut(path).unwrap() = replacement;
        assert_eq!(
            metadata.verify_network(Some(environment), &altered),
            Err(expected),
            "{path}"
        );
    }
    let mut altered = valid;
    altered["Options"]["com.docker.network.bridge.trusted_host_interfaces"] = json!("eth0");
    assert_eq!(
        metadata.verify_network(Some(environment), &altered),
        Err(EnvironmentError::PolicyMismatch)
    );
}

fn proxy(metadata: &EgressMetadata) -> Value {
    let body = serde_json::to_value(metadata.proxy_body()).unwrap();
    json!({"Id":"c".repeat(64), "Image":metadata.image_id, "Name":format!("/{}",metadata.proxy_name()),
        "Config":body, "HostConfig":body["HostConfig"], "Mounts":[]})
}

#[test]
fn proxy_refuses_mounts_ports_forwarding_and_privilege_drift() {
    let metadata = metadata();
    let valid = proxy(&metadata);
    assert!(metadata.verify_proxy(&valid).is_ok());
    for (path, replacement) in [
        ("/Mounts", json!([{"Source":"/var/run/docker.sock"}])),
        ("/HostConfig/Sysctls/net.ipv4.ip_forward", json!("1")),
        ("/HostConfig/Privileged", json!(true)),
        ("/HostConfig/ReadonlyRootfs", json!(false)),
        ("/HostConfig/CapAdd", json!(["ALL"])),
        ("/Config/Entrypoint", json!(["/bin/sh"])),
    ] {
        let mut altered = valid.clone();
        *altered.pointer_mut(path).unwrap() = replacement;
        assert_eq!(
            metadata.verify_proxy(&altered),
            Err(EnvironmentError::PolicyMismatch),
            "{path}"
        );
    }
    let mut altered = valid;
    altered["HostConfig"]["PortBindings"] = json!({"3128/tcp":[{"HostPort":"3128"}]});
    assert_eq!(
        metadata.verify_proxy(&altered),
        Err(EnvironmentError::PolicyMismatch)
    );
}

#[test]
fn metadata_cannot_inject_firewall_rules_or_select_mutable_image() {
    let valid = metadata();
    let mut raw = json!({"schemaVersion":1,"installationId":valid.installation_id.to_string(),"imageId":valid.image_id,"hostIpv4":["127.0.0.1; accept"]});
    assert!(serde_json::from_value::<EgressMetadata>(raw.clone()).is_err());
    raw["hostIpv4"] = json!(["127.0.0.1"]);
    raw["imageId"] = json!("squid:latest");
    assert_eq!(
        serde_json::from_value::<EgressMetadata>(raw)
            .unwrap()
            .validate(),
        Err(EnvironmentError::InvalidConfiguration)
    );
}
