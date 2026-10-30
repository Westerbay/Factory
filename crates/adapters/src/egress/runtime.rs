use super::{EgressMetadata, UserNetwork, policy::*};
use bollard::{Docker, errors::Error, query_parameters::*};
use factory_core::environments::{Environment, EnvironmentError};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Used only by the installation's exclusive supervisor, never by a browser-supplied ID.
pub struct DockerEgress {
    docker: Docker,
    metadata: EgressMetadata,
    permitted: Arc<dyn Fn() -> bool + Send + Sync>,
    serial: Mutex<()>,
}

impl DockerEgress {
    pub fn new(
        docker: Docker,
        metadata: EgressMetadata,
        permitted: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, EnvironmentError> {
        metadata.validate()?;
        Ok(Self {
            docker,
            metadata,
            permitted,
            serial: Mutex::new(()),
        })
    }

    fn authorize(&self) -> Result<(), EnvironmentError> {
        if (self.permitted)() {
            Ok(())
        } else {
            Err(EnvironmentError::Unavailable)
        }
    }

    /// Idempotently prepares new policy-3 resources. It never replaces or attaches a user container.
    pub async fn prepare_user_network(
        &self,
        environment: &Environment,
    ) -> Result<UserNetwork, EnvironmentError> {
        if environment.installation_id != self.metadata.installation_id || environment.id.is_nil() {
            return Err(EnvironmentError::OwnershipConflict);
        }
        tokio::time::timeout(Duration::from_secs(60), async {
            let _serial = self.serial.lock().await;
            self.authorize()?;
            self.require_engine().await?;
            let outbound = self.network(None, true).await?;
            let proxy = self.proxy(&outbound).await?;
            let network = self.network(Some(environment.id), true).await?;
            self.verify_members(&network, Some(&proxy), Some(environment))
                .await?;
            let network_id = id(&network)?;
            let proxy_id = id(&proxy)?;
            if network["Containers"].get(proxy_id).is_none() {
                self.authorize()?;
                self.docker
                    .connect_network(
                        network_id,
                        serde_json::from_value(json!({
                            "Container": proxy_id, "EndpointConfig": {"Aliases": [PROXY_ALIAS]}
                        }))
                        .expect("fixed connect contract"),
                    )
                    .await
                    .map_err(map_error)?;
            }
            let network = self.network(Some(environment.id), false).await?;
            self.verify_members(&network, Some(&proxy), Some(environment))
                .await?;
            self.verify_proxy_networks(&self.inspect_proxy().await?)
                .await?;
            Ok(UserNetwork {
                id: id(&network)?.to_owned(),
                name: self.metadata.user_network_name(environment.id),
                proxy_url: PROXY_URL,
            })
        })
        .await
        .map_err(|_| EnvironmentError::Timeout)?
    }

    /// Network ownership inspection does not depend on proxy health, preserving local terminal access.
    pub async fn inspect_user_network(
        &self,
        environment: &Environment,
    ) -> Result<UserNetwork, EnvironmentError> {
        if environment.installation_id != self.metadata.installation_id {
            return Err(EnvironmentError::OwnershipConflict);
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            let network = self.network(Some(environment.id), false).await?;
            let proxy = match self
                .docker
                .inspect_container(&self.metadata.proxy_name(), None::<InspectContainerOptions>)
                .await
            {
                Ok(value) => {
                    let value =
                        serde_json::to_value(value).map_err(|_| EnvironmentError::Unavailable)?;
                    self.metadata.verify_proxy(&value)?;
                    Some(value)
                }
                Err(Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => None,
                Err(error) => return Err(map_error(error)),
            };
            self.verify_members(&network, proxy.as_ref(), Some(environment))
                .await?;
            Ok(UserNetwork {
                id: id(&network)?.to_owned(),
                name: self.metadata.user_network_name(environment.id),
                proxy_url: PROXY_URL,
            })
        })
        .await
        .map_err(|_| EnvironmentError::Timeout)?
    }

    pub async fn available(&self) -> Result<bool, EnvironmentError> {
        let proxy = tokio::time::timeout(Duration::from_secs(10), self.inspect_proxy())
            .await
            .map_err(|_| EnvironmentError::Timeout)??;
        Ok(proxy["State"]["Running"] == true
            && proxy["State"]["Restarting"] != true
            && proxy["State"]["Paused"] != true)
    }

    async fn require_engine(&self) -> Result<(), EnvironmentError> {
        let version = self.docker.version().await.map_err(map_error)?;
        let value = serde_json::to_value(version).map_err(|_| EnvironmentError::Unavailable)?;
        let api = value["ApiVersion"].as_str().unwrap_or("");
        let parts = api
            .split('.')
            .filter_map(|n| n.parse::<u32>().ok())
            .collect::<Vec<_>>();
        if value["Os"] != "linux" || parts.len() != 2 || (parts[0], parts[1]) < (1, 48) {
            return Err(EnvironmentError::PolicyMismatch);
        }
        Ok(())
    }

    async fn network(
        &self,
        environment: Option<Uuid>,
        create: bool,
    ) -> Result<Value, EnvironmentError> {
        let name = environment.map_or_else(
            || self.metadata.outbound_name(),
            |id| self.metadata.user_network_name(id),
        );
        let inspected = self
            .docker
            .inspect_network(&name, None::<InspectNetworkOptions>)
            .await;
        let network = match inspected {
            Ok(network) => network,
            Err(Error::DockerResponseServerError {
                status_code: 404, ..
            }) if create => {
                self.authorize()?;
                let result = self
                    .docker
                    .create_network(self.metadata.network_body(environment))
                    .await;
                match result {
                    Ok(response) if response.warning.is_empty() => (),
                    Ok(_) => return Err(EnvironmentError::PolicyMismatch),
                    Err(Error::DockerResponseServerError {
                        status_code: 409, ..
                    }) => (),
                    Err(e) => return Err(map_error(e)),
                }
                self.docker
                    .inspect_network(&name, None::<InspectNetworkOptions>)
                    .await
                    .map_err(map_error)?
            }
            Err(e) => return Err(map_error(e)),
        };
        let value = serde_json::to_value(network).map_err(|_| EnvironmentError::Unavailable)?;
        self.metadata.verify_network(environment, &value)?;
        Ok(value)
    }

    async fn inspect_proxy(&self) -> Result<Value, EnvironmentError> {
        let value = self
            .docker
            .inspect_container(&self.metadata.proxy_name(), None::<InspectContainerOptions>)
            .await
            .map_err(map_error)?;
        let value = serde_json::to_value(value).map_err(|_| EnvironmentError::Unavailable)?;
        self.metadata.verify_proxy(&value)?;
        Ok(value)
    }

    async fn proxy(&self, outbound: &Value) -> Result<Value, EnvironmentError> {
        let image = self
            .docker
            .inspect_image(&self.metadata.image_id)
            .await
            .map_err(map_error)?;
        let image = serde_json::to_value(image).map_err(|_| EnvironmentError::Unavailable)?;
        if image["Id"] != self.metadata.image_id
            || image["Os"] != "linux"
            || image["Config"]["Labels"]["io.factory.egress.image"] != "1"
            || image["Config"]["Labels"]["io.factory.egress.squid"] != "7.7"
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        match self
            .docker
            .inspect_container(&self.metadata.proxy_name(), None::<InspectContainerOptions>)
            .await
        {
            Ok(_) => (),
            Err(Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                self.authorize()?;
                let result = self
                    .docker
                    .create_container(
                        Some(CreateContainerOptions {
                            name: Some(self.metadata.proxy_name()),
                            ..Default::default()
                        }),
                        self.metadata.proxy_body(),
                    )
                    .await;
                match result {
                    Ok(response) if response.warnings.is_empty() => (),
                    Ok(_) => return Err(EnvironmentError::PolicyMismatch),
                    Err(Error::DockerResponseServerError {
                        status_code: 409, ..
                    }) => (),
                    Err(e) => return Err(map_error(e)),
                }
            }
            Err(e) => return Err(map_error(e)),
        }
        let proxy = self.inspect_proxy().await?;
        self.verify_members(outbound, Some(&proxy), None).await?;
        self.verify_proxy_networks(&proxy).await?;
        if proxy["State"]["Running"] != true {
            self.authorize()?;
            self.docker
                .start_container(id(&proxy)?, None::<StartContainerOptions>)
                .await
                .map_err(map_error)?;
        }
        self.inspect_proxy().await
    }

    async fn verify_members(
        &self,
        network: &Value,
        proxy: Option<&Value>,
        environment: Option<&Environment>,
    ) -> Result<(), EnvironmentError> {
        if let Some(members) = network["Containers"].as_object() {
            for (container_id, endpoint) in members {
                if proxy.is_some_and(|proxy| proxy["Id"].as_str() == Some(container_id.as_str())) {
                    continue;
                }
                let Some(environment) = environment else {
                    return Err(EnvironmentError::OwnershipConflict);
                };
                if endpoint["Name"] != environment.container_name {
                    return Err(EnvironmentError::OwnershipConflict);
                }
                let container = self
                    .docker
                    .inspect_container(container_id, None::<InspectContainerOptions>)
                    .await
                    .map_err(map_error)?;
                let container =
                    serde_json::to_value(container).map_err(|_| EnvironmentError::Unavailable)?;
                if container["Id"] != *container_id
                    || container["Config"]["Labels"]["io.factory.installation"]
                        != environment.installation_id.to_string()
                    || container["Config"]["Labels"]["io.factory.environment"]
                        != environment.id.to_string()
                    || container["Config"]["Labels"]["io.factory.user"]
                        != environment.user_id.to_string()
                {
                    return Err(EnvironmentError::OwnershipConflict);
                }
            }
        }
        Ok(())
    }

    async fn verify_proxy_networks(&self, proxy: &Value) -> Result<(), EnvironmentError> {
        let networks = proxy["NetworkSettings"]["Networks"]
            .as_object()
            .ok_or(EnvironmentError::PolicyMismatch)?;
        for (name, endpoint) in networks {
            let network = self
                .docker
                .inspect_network(name, None::<InspectNetworkOptions>)
                .await
                .map_err(map_error)?;
            let network =
                serde_json::to_value(network).map_err(|_| EnvironmentError::Unavailable)?;
            let environment = if name == &self.metadata.outbound_name() {
                None
            } else {
                Some(
                    Uuid::parse_str(
                        network["Labels"]["io.factory.environment"]
                            .as_str()
                            .ok_or(EnvironmentError::OwnershipConflict)?,
                    )
                    .map_err(|_| EnvironmentError::OwnershipConflict)?,
                )
            };
            self.metadata.verify_network(environment, &network)?;
            if endpoint["NetworkID"] != network["Id"]
                && !(proxy["State"]["Running"] == false && endpoint["NetworkID"] == "")
            {
                return Err(EnvironmentError::OwnershipConflict);
            }
        }
        Ok(())
    }
}

fn id(value: &Value) -> Result<&str, EnvironmentError> {
    value["Id"]
        .as_str()
        .filter(|value| docker_id(value))
        .ok_or(EnvironmentError::OwnershipConflict)
}
fn map_error(error: Error) -> EnvironmentError {
    match error {
        Error::RequestTimeoutError => EnvironmentError::Timeout,
        _ => EnvironmentError::Unavailable,
    }
}
