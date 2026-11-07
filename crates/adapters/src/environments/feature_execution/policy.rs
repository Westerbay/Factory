use super::super::policy::{empty, labels};
use super::CHECKOUT;
use bollard::models::ContainerCreateBody;
use factory_core::{
    conversations::execution::ExecutionError,
    environments::{Environment, EnvironmentError},
    features::{TaskWorkspace, WorkspaceAccess},
    projects::Project,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

pub(super) const ROLE: &str = "io.factory.feature.worker";
const ATTEMPT: &str = "io.factory.feature.attempt";
const FEATURE: &str = "io.factory.feature.id";
const PROJECT: &str = "io.factory.feature.project";
const GENERATION: &str = "io.factory.feature.generation";

#[derive(Clone, Copy)]
pub(super) enum Role {
    Agent,
    Check,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Check => "check",
        }
    }
}

pub(super) struct WorkerIdentity {
    pub(super) name: String,
    labels: HashMap<String, String>,
    pub(super) checkout: String,
    relay: Option<String>,
    readonly: bool,
}

impl WorkerIdentity {
    pub(super) fn new(
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        generation: i64,
        role: Role,
    ) -> Result<Self, ExecutionError> {
        if project.id.is_nil()
            || workspace.feature_id.is_nil()
            || workspace.attempt_id.is_nil()
            || project.user_id != env.user_id
            || project.environment_id != env.id
            || generation < 1
            || generation > env.generation
        {
            return Err(ExecutionError::Invalid);
        }
        let mut labels = labels(env);
        labels.extend([
            (ROLE.into(), role.as_str().into()),
            (ATTEMPT.into(), workspace.attempt_id.to_string()),
            (PROJECT.into(), project.id.to_string()),
            (FEATURE.into(), workspace.feature_id.to_string()),
            (GENERATION.into(), generation.to_string()),
        ]);
        Ok(Self {
            name: worker_name(env, workspace.attempt_id, role.as_str()),
            labels,
            checkout: format!(
                ".factory/features/{}/{}/attempts/{}/checkout",
                project.id, workspace.feature_id, workspace.attempt_id
            ),
            relay: matches!(role, Role::Agent)
                .then(|| format!(".factory/feature-relays/{}", workspace.attempt_id)),
            readonly: workspace.access == WorkspaceAccess::ReadOnly,
        })
    }

    pub(super) fn body(
        &self,
        env: &Environment,
        cwd: &str,
        command: Vec<String>,
    ) -> Result<ContainerCreateBody, ExecutionError> {
        let mut mounts = vec![mount(env, CHECKOUT, &self.checkout, self.readonly)];
        if let Some(relay) = &self.relay {
            mounts.push(mount(env, "/run/factory-relay", relay, true));
        }
        serde_json::from_value(json!({
            "Image": env.image_id, "User": "10000:10000", "WorkingDir": cwd,
            "Entrypoint": command, "Cmd": [], "Labels": self.labels,
            "OpenStdin": true, "StdinOnce": true, "AttachStdin": true,
            "AttachStdout": true, "AttachStderr": true, "Tty": false,
            "Env": ["HOME=/home/factory", "CODEX_HOME=/home/factory/.codex",
                "CLAUDE_CONFIG_DIR=/home/factory/.claude", "GIT_CONFIG_NOSYSTEM=1",
                "PATH=/usr/local/bin:/usr/bin:/bin", "NO_PROXY=", "no_proxy="],
            "HostConfig": {
                "NetworkMode": "none", "ReadonlyRootfs": true, "CapDrop": ["ALL"],
                "SecurityOpt": ["no-new-privileges"], "Init": true, "PidsLimit": 256,
                "Privileged": false, "PublishAllPorts": false,
                "IpcMode": "private", "CgroupnsMode": "private", "Runtime": "runc",
                "RestartPolicy": {"Name": "no"}, "AutoRemove": false, "Mounts": mounts,
                "Tmpfs": {
                    "/tmp": "rw,nosuid,nodev,size=268435456,mode=1777",
                    "/home/factory": "rw,nosuid,nodev,size=268435456,mode=0700,uid=10000,gid=10000"
                },
                "LogConfig": {"Type": "none", "Config": {}}
            }
        }))
        .map_err(|_| ExecutionError::PreflightUnavailable)
    }
}

fn mount(env: &Environment, target: &str, subpath: &str, readonly: bool) -> Value {
    json!({"Type": "volume", "Source": env.volume_name, "Target": target,
        "ReadOnly": readonly, "VolumeOptions": {"NoCopy": true, "Subpath": subpath}})
}

fn worker_name(env: &Environment, attempt: Uuid, role: &str) -> String {
    format!("{}-feature-{}-{role}", env.container_name, attempt.simple())
}

pub(super) fn inspect_id(value: &Value) -> Result<&str, EnvironmentError> {
    value["Id"]
        .as_str()
        .filter(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or(EnvironmentError::OwnershipConflict)
}

pub(super) fn verify_worker(value: &Value, expected: &Value) -> Result<(), ExecutionError> {
    let host = &value["HostConfig"];
    let expected_host = &expected["HostConfig"];
    let config = &value["Config"];
    for key in [
        "Image",
        "User",
        "WorkingDir",
        "Entrypoint",
        "OpenStdin",
        "StdinOnce",
        "Tty",
        "Env",
    ] {
        if config[key] != expected[key] {
            return Err(ExecutionError::PreflightUnavailable);
        }
    }
    if !empty(&config["Cmd"]) {
        return Err(ExecutionError::PreflightUnavailable);
    }
    for key in [
        "NetworkMode",
        "ReadonlyRootfs",
        "CapDrop",
        "Init",
        "PidsLimit",
        "Privileged",
        "PublishAllPorts",
        "IpcMode",
        "CgroupnsMode",
        "Runtime",
        "AutoRemove",
        "Tmpfs",
        "LogConfig",
    ] {
        if host[key] != expected_host[key] {
            return Err(ExecutionError::PreflightUnavailable);
        }
    }
    for key in [
        "Binds",
        "VolumesFrom",
        "CapAdd",
        "Devices",
        "DeviceRequests",
        "PortBindings",
        "Links",
        "GroupAdd",
        "Sysctls",
        "PidMode",
        "UsernsMode",
    ] {
        if !empty(&host[key]) {
            return Err(ExecutionError::PreflightUnavailable);
        }
    }
    if host["RestartPolicy"]["Name"] != "no"
        || (host["SecurityOpt"] != json!(["no-new-privileges"])
            && host["SecurityOpt"] != json!(["no-new-privileges=true"]))
        || value["NetworkSettings"]["Networks"]
            .as_object()
            .is_none_or(|networks| networks.keys().any(|name| name != "none"))
    {
        return Err(ExecutionError::PreflightUnavailable);
    }
    let mounts = expected_host["Mounts"]
        .as_array()
        .ok_or(ExecutionError::PreflightUnavailable)?;
    let actual = host["Mounts"]
        .as_array()
        .ok_or(ExecutionError::PreflightUnavailable)?;
    let attached = value["Mounts"]
        .as_array()
        .ok_or(ExecutionError::PreflightUnavailable)?;
    if actual.len() != mounts.len() || attached.len() != mounts.len() {
        return Err(ExecutionError::PreflightUnavailable);
    }
    for wanted in mounts {
        if !actual.iter().any(|mount| {
            ["Type", "Source", "Target", "VolumeOptions"]
                .into_iter()
                .all(|key| mount[key] == wanted[key])
                && mount["ReadOnly"].as_bool().unwrap_or(false) == wanted["ReadOnly"]
        }) || !attached.iter().any(|mount| {
            mount["Type"] == wanted["Type"]
                && mount["Name"] == wanted["Source"]
                && mount["Destination"] == wanted["Target"]
                && mount["RW"].as_bool() == wanted["ReadOnly"].as_bool().map(|ro| !ro)
        }) {
            return Err(ExecutionError::PreflightUnavailable);
        }
    }
    Ok(())
}

pub(super) fn verify_identity(
    env: &Environment,
    identity: &WorkerIdentity,
    value: &Value,
) -> Result<(), ExecutionError> {
    if value["Name"] != format!("/{}", identity.name)
        || value["Image"] != env.image_id
        || identity
            .labels
            .iter()
            .any(|(key, label)| value["Config"]["Labels"][key] != *label)
    {
        return Err(ExecutionError::CleanupUncertain);
    }
    Ok(())
}

pub(super) fn verify_worker_owner(
    env: &Environment,
    value: &Value,
    id: &str,
) -> Result<(), EnvironmentError> {
    let labels = &value["Config"]["Labels"];
    let role = labels[ROLE]
        .as_str()
        .filter(|s| matches!(*s, "agent" | "check"))
        .ok_or(EnvironmentError::OwnershipConflict)?;
    let attempt = labels[ATTEMPT]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(EnvironmentError::OwnershipConflict)?;
    if inspect_id(value)? != id
        || value["Name"] != format!("/{}", worker_name(env, attempt, role))
        || labels["io.factory.installation"] != env.installation_id.to_string()
        || labels["io.factory.environment"] != env.id.to_string()
        || labels["io.factory.user"] != env.user_id.to_string()
    {
        return Err(EnvironmentError::OwnershipConflict);
    }
    Ok(())
}
