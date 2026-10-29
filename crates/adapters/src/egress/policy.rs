use bollard::models::{ContainerCreateBody, NetworkCreateRequest};
use factory_core::environments::EnvironmentError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::File, io::Read, net::Ipv4Addr, path::Path};
use uuid::Uuid;

pub const PROXY_ALIAS: &str = "factory-egress";
pub const PROXY_URL: &str = "http://factory-egress:3128";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressMetadata {
    pub schema_version: u32,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub installation_id: Uuid,
    pub image_id: String,
    pub host_ipv4: Vec<Ipv4Addr>,
}

impl EgressMetadata {
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, EnvironmentError> {
        let mut bytes = Vec::new();
        File::open(directory.as_ref().join("egress.json"))
            .map_err(|_| EnvironmentError::Unavailable)?
            .take(16385)
            .read_to_end(&mut bytes)
            .map_err(|_| EnvironmentError::Unavailable)?;
        if bytes.len() > 16384 {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|_| EnvironmentError::InvalidConfiguration)?;
        value.validate()?;
        Ok(value)
    }

    pub(super) fn validate(&self) -> Result<(), EnvironmentError> {
        if self.schema_version != 1
            || self.installation_id.is_nil()
            || !self.image_id.strip_prefix("sha256:").is_some_and(docker_id)
            || self.host_ipv4.is_empty()
            || self.host_ipv4.len() > 128
        {
            return Err(EnvironmentError::InvalidConfiguration);
        }
        Ok(())
    }

    pub fn proxy_name(&self) -> String {
        format!("factory-egress-{}", self.installation_id)
    }
    pub fn outbound_name(&self) -> String {
        format!("factory-outbound-{}", self.installation_id)
    }

    pub(super) fn labels(&self, environment: Option<Uuid>) -> Value {
        let mut labels = json!({"io.factory.installation": self.installation_id.to_string(), "io.factory.egress.policy": "1"});
        labels["io.factory.egress.role"] = json!(if environment.is_some() {
            "user-network"
        } else {
            "shared"
        });
        if let Some(id) = environment {
            labels["io.factory.environment"] = json!(id.to_string());
        }
        labels
    }

    pub fn user_network_name(&self, environment: Uuid) -> String {
        format!("factory-net-{}-{}", self.installation_id, environment)
    }

    pub(super) fn network_body(&self, environment: Option<Uuid>) -> NetworkCreateRequest {
        let internal = environment.is_some();
        serde_json::from_value(json!({
            "Name": environment.map_or_else(|| self.outbound_name(), |id| self.user_network_name(id)),
            "Driver": "bridge", "Internal": internal, "Attachable": false, "Ingress": false,
            "EnableIPv6": false, "Labels": self.labels(environment),
            "Options": if internal { json!({"com.docker.network.bridge.gateway_mode_ipv4": "isolated", "com.docker.network.enable_ipv4": "true"}) } else { json!({"com.docker.network.enable_ipv4": "true"}) }
        })).expect("fixed network contract")
    }

    pub(super) fn verify_network(
        &self,
        environment: Option<Uuid>,
        actual: &Value,
    ) -> Result<(), EnvironmentError> {
        let expected =
            serde_json::to_value(self.network_body(environment)).expect("network serializes");
        if !actual["Id"].as_str().is_some_and(docker_id) || actual["Labels"] != expected["Labels"] {
            return Err(EnvironmentError::OwnershipConflict);
        }
        for key in [
            "Name",
            "Driver",
            "Internal",
            "Attachable",
            "Ingress",
            "EnableIPv6",
        ] {
            if actual[key] != expected[key] {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        if actual["Scope"] != "local"
            || actual["IPAM"]["Driver"] != "default"
            || !empty(&actual["IPAM"]["Options"])
            || actual["Options"] != expected["Options"]
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        Ok(())
    }

    pub(super) fn proxy_body(&self) -> ContainerCreateBody {
        let mut labels = self.labels(None);
        labels["io.factory.egress.image"] = json!("1");
        labels["io.factory.egress.squid"] = json!("7.7");
        let hosts = self
            .host_ipv4
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        serde_json::from_value(json!({
            "Image": self.image_id, "User": "0:0", "Entrypoint": ["/usr/local/bin/factory-egress"], "Cmd": [],
            "Env": [format!("FACTORY_HOST_IPV4={hosts}"), "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
            "Labels": labels,
            "HostConfig": {
                "NetworkMode": self.outbound_name(), "ReadonlyRootfs": true, "Init": false,
                "Privileged": false, "PublishAllPorts": false, "AutoRemove": false,
                "IpcMode": "private", "CgroupnsMode": "private", "Runtime": "runc",
                "CapDrop": ["ALL"], "CapAdd": ["NET_ADMIN", "SETUID", "SETGID", "SETPCAP", "CHOWN"],
                "SecurityOpt": ["no-new-privileges"], "RestartPolicy": {"Name": "unless-stopped"},
                "Sysctls": {"net.ipv4.ip_forward": "0", "net.ipv6.conf.all.forwarding": "0", "net.ipv6.conf.all.disable_ipv6": "1"},
                "Tmpfs": {"/tmp": "rw,noexec,nosuid,nodev,size=16777216,mode=1777"},
                "LogConfig": {"Type": "none", "Config": {}}
            }
        })).expect("fixed proxy contract")
    }

    pub(super) fn verify_proxy(&self, value: &Value) -> Result<(), EnvironmentError> {
        let expected = serde_json::to_value(self.proxy_body()).expect("proxy serializes");
        if value["Config"]["Labels"] != expected["Labels"]
            || value["Name"] != format!("/{}", self.proxy_name())
            || !value["Id"].as_str().is_some_and(docker_id)
        {
            return Err(EnvironmentError::OwnershipConflict);
        }
        if value["Image"] != self.image_id {
            return Err(EnvironmentError::PolicyMismatch);
        }
        for key in ["Image", "User", "Entrypoint", "Env"] {
            if value["Config"][key] != expected[key] {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        if !empty(&value["Config"]["Cmd"])
            || !empty(&value["Mounts"])
            || !empty(&value["Config"]["ExposedPorts"])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        for key in [
            "NetworkMode",
            "ReadonlyRootfs",
            "Init",
            "Privileged",
            "PublishAllPorts",
            "AutoRemove",
            "CapDrop",
            "CapAdd",
            "Sysctls",
            "Tmpfs",
            "LogConfig",
            "IpcMode",
            "CgroupnsMode",
            "Runtime",
        ] {
            if value["HostConfig"][key] != expected["HostConfig"][key] {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        if value["HostConfig"]["RestartPolicy"]["Name"] != "unless-stopped"
            || !matches!(value["HostConfig"]["SecurityOpt"].as_array(), Some(options) if options == &vec![json!("no-new-privileges")] || options == &vec![json!("no-new-privileges=true")])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        for key in [
            "Binds",
            "VolumesFrom",
            "PortBindings",
            "Devices",
            "DeviceRequests",
            "Links",
            "GroupAdd",
            "Dns",
            "DnsOptions",
            "DnsSearch",
            "ExtraHosts",
        ] {
            if !empty(&value["HostConfig"][key]) {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        for key in ["PidMode", "UTSMode"] {
            if value["HostConfig"][key]
                .as_str()
                .is_some_and(|v| !v.is_empty())
            {
                return Err(EnvironmentError::PolicyMismatch);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct UserNetwork {
    pub id: String,
    pub name: String,
    pub proxy_url: &'static str,
}

pub(super) fn empty(value: &Value) -> bool {
    value.is_null()
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}
pub(super) fn docker_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn deserialize_uuid<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let value = String::deserialize(deserializer)?;
    Uuid::parse_str(&value).map_err(serde::de::Error::custom)
}
