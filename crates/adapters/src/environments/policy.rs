use super::preparation::{POLICY_VERSION, immutable_image};
use crate::egress::{PROXY_URL, user_network_name};
use bollard::models::ContainerCreateBody;
use factory_core::environments::{Environment, EnvironmentError};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

const HOME: &str = "/home/factory";
const USER: &str = "10000:10000";
// Docker can normalize PidsLimit=-1 to null, leaving systemd's DefaultTasksMax active.
// The OCI annotation sets infinity for this container's scope without changing the host.
const SYSTEMD_UNLIMITED_TASKS: &str = "uint64 18446744073709551615";
const TMPFS: &str = "rw,noexec,nosuid,nodev,size=67108864,mode=1777";
const MASKED_PATHS: &[&str] = &[
    "/proc/acpi",
    "/proc/asound",
    "/proc/interrupts",
    "/proc/kcore",
    "/proc/keys",
    "/proc/latency_stats",
    "/proc/sched_debug",
    "/proc/scsi",
    "/proc/timer_list",
    "/proc/timer_stats",
    "/sys/devices/virtual/powercap",
    "/sys/firmware",
];
const READONLY_PATHS: &[&str] = &[
    "/proc/bus",
    "/proc/fs",
    "/proc/irq",
    "/proc/sys",
    "/proc/sysrq-trigger",
];

#[derive(Clone, Debug)]
pub struct IsolationPolicy;

impl IsolationPolicy {
    pub(super) fn create_body(&self, env: &Environment) -> ContainerCreateBody {
        serde_json::from_value(json!({
            "Image": env.image_id, "User": USER, "WorkingDir": "/home/factory/Documents",
            "Env": ["HOME=/home/factory", "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                format!("HTTP_PROXY={PROXY_URL}"), format!("HTTPS_PROXY={PROXY_URL}"),
                format!("http_proxy={PROXY_URL}"), format!("https_proxy={PROXY_URL}"), "NO_PROXY=", "no_proxy="],
            "Entrypoint": ["/bin/sleep"], "Cmd": ["infinity"], "NetworkDisabled": false,
            "Labels": labels(env),
            "HostConfig": {
                "Memory": 0, "MemorySwap": 0, "NanoCpus": 0,
                "PidsLimit": -1, "ReadonlyRootfs": true, "Init": true,
                "Annotations": {"org.systemd.property.TasksMax": SYSTEMD_UNLIMITED_TASKS},
                "CapDrop": ["ALL"], "SecurityOpt": ["no-new-privileges"],
                "NetworkMode": user_network_name(env.installation_id, env.id), "Privileged": false, "PublishAllPorts": false,
                "IpcMode": "private", "CgroupnsMode": "private", "AutoRemove": false,
                "MaskedPaths": MASKED_PATHS, "ReadonlyPaths": READONLY_PATHS, "Runtime": "runc",
                "RestartPolicy": {"Name": "no"},
                "Tmpfs": {"/tmp": TMPFS},
                "LogConfig": {"Type": "json-file", "Config": {"max-size": "1m", "max-file": "2"}},
                "Mounts": [{"Type": "volume", "Source": env.volume_name, "Target": HOME, "ReadOnly": false}]
            }
        })).expect("fixed Docker create contract")
    }

    pub(super) fn verify_container(
        &self,
        env: &Environment,
        value: &Value,
    ) -> Result<(), EnvironmentError> {
        let expected =
            serde_json::to_value(self.create_body(env)).expect("Docker contract serializes");
        let config = &value["Config"];
        let host = &value["HostConfig"];
        for key in ["User", "WorkingDir", "Env", "Entrypoint", "Cmd"] {
            if config[key] != expected[key] {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        // Docker omits NetworkDisabled when false on inspect. Only that exact
        // normalization is accepted; interface identity is verified separately.
        if !config["NetworkDisabled"].is_null() && config["NetworkDisabled"] != false {
            return Err(EnvironmentError::PolicyMismatch);
        }
        if value["Image"] != env.image_id || config["Image"] != env.image_id {
            return Err(EnvironmentError::PolicyMismatch);
        }
        for key in [
            "Memory",
            "MemorySwap",
            "NanoCpus",
            "Annotations",
            "ReadonlyRootfs",
            "Init",
            "CapDrop",
            "NetworkMode",
            "Privileged",
            "PublishAllPorts",
            "IpcMode",
            "CgroupnsMode",
            "AutoRemove",
            "Tmpfs",
            "LogConfig",
            "MaskedPaths",
            "ReadonlyPaths",
            "Runtime",
        ] {
            if host[key] != expected["HostConfig"][key] {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        // Docker normalizes an explicitly unlimited PID limit to null on inspect.
        if !host["PidsLimit"].is_null() && host["PidsLimit"] != -1 {
            return Err(EnvironmentError::PolicyMismatch);
        }
        let security = &host["SecurityOpt"];
        if security != &json!(["no-new-privileges"])
            && security != &json!(["no-new-privileges=true"])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        if host["RestartPolicy"]["Name"] != "no" {
            return Err(EnvironmentError::PolicyMismatch);
        }
        for key in [
            "Binds",
            "VolumesFrom",
            "CapAdd",
            "Devices",
            "DeviceRequests",
            "DeviceCgroupRules",
            "PortBindings",
            "Links",
            "GroupAdd",
            "Sysctls",
            "Dns",
            "DnsOptions",
            "DnsSearch",
            "ExtraHosts",
        ] {
            if !empty(&host[key]) {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        for key in [
            "PidMode",
            "UTSMode",
            "UsernsMode",
            "CgroupParent",
            "ContainerIDFile",
        ] {
            if !empty(&host[key]) {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        for key in ["ExposedPorts", "Volumes", "Healthcheck"] {
            if !empty(&config[key]) {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        let mounts = value["Mounts"]
            .as_array()
            .ok_or(EnvironmentError::PolicyMismatch)?;
        if mounts.len() != 1
            || mounts[0]["Type"] != "volume"
            || mounts[0]["Name"] != env.volume_name
            || mounts[0]["Destination"] != HOME
            || mounts[0]["RW"] != true
            || mounts[0]["Driver"] != "local"
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        let configured = host["Mounts"]
            .as_array()
            .ok_or(EnvironmentError::PolicyMismatch)?;
        if configured.len() != 1
            || configured[0]["Type"] != "volume"
            || configured[0]["Source"] != env.volume_name
            || configured[0]["Target"] != HOME
            || !(configured[0]["ReadOnly"].is_null() || configured[0]["ReadOnly"] == false)
            || !empty(&configured[0]["VolumeOptions"])
            || !empty(&configured[0]["BindOptions"])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        let network_name = user_network_name(env.installation_id, env.id);
        let networks = value["NetworkSettings"]["Networks"]
            .as_object()
            .ok_or(EnvironmentError::PolicyMismatch)?;
        if networks.len() != 1 || !networks.contains_key(&network_name) {
            return Err(EnvironmentError::PolicyMismatch);
        }
        if !empty(&value["NetworkSettings"]["Ports"]) {
            return Err(EnvironmentError::PolicyMismatch);
        }
        Ok(())
    }
}

pub(super) fn empty(value: &Value) -> bool {
    value.is_null()
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
        || value.as_str() == Some("")
}

pub(super) fn validate_record(
    env: &Environment,
    installation: Uuid,
) -> Result<(), EnvironmentError> {
    let name = format!("factory-{}-{}", installation.simple(), env.id.simple());
    if env.installation_id != installation
        || env.id.is_nil()
        || env.user_id.is_nil()
        || env.container_name != name
        || env.volume_name != format!("{name}-home")
        || !immutable_image(&env.image_id)
    {
        return Err(EnvironmentError::OwnershipConflict);
    }
    Ok(())
}

pub(super) fn labels(env: &Environment) -> HashMap<String, String> {
    [
        ("io.factory.installation", env.installation_id.to_string()),
        ("io.factory.environment", env.id.to_string()),
        ("io.factory.user", env.user_id.to_string()),
        ("io.factory.policy", env.policy_version.clone()),
        ("io.factory.image", env.image_id.clone()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

pub(super) fn verify_labels(env: &Environment, actual: &Value) -> Result<(), EnvironmentError> {
    if labels(env).iter().any(|(key, value)| actual[key] != *value) {
        return Err(EnvironmentError::OwnershipConflict);
    }
    Ok(())
}

pub(super) fn require_current_policy(env: &Environment) -> Result<(), EnvironmentError> {
    if env.policy_version == POLICY_VERSION {
        Ok(())
    } else {
        Err(EnvironmentError::PolicyMismatch)
    }
}
