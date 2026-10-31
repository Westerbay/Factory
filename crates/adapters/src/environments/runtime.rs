use super::{IsolationPolicy, PreparationMetadata, policy::*};
use crate::egress::{DockerEgress, EgressMetadata};
use async_trait::async_trait;
use bollard::{
    API_DEFAULT_VERSION, Docker, errors::Error, models::VolumeCreateRequest, query_parameters::*,
};
use factory_core::environments::*;
use serde::Serialize;
use serde_json::Value;
use std::{future::Future, sync::Arc, time::Duration};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

pub struct DockerEnvironmentRuntime {
    pub(super) docker: Docker,
    pub(super) metadata: PreparationMetadata,
    pub(super) policy: IsolationPolicy,
    pub(super) timeout: Duration,
    pub(super) mutation_permitted: Arc<dyn Fn() -> bool + Send + Sync>,
    pub(super) egress: Arc<DockerEgress>,
}

impl DockerEnvironmentRuntime {
    pub fn connect(
        metadata: PreparationMetadata,
        egress_metadata: EgressMetadata,
        mutation_permitted: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, EnvironmentError> {
        metadata.validate()?;
        if egress_metadata.installation_id != metadata.installation_id() {
            return Err(EnvironmentError::OwnershipConflict);
        }
        // Explicit endpoints. Never use *_defaults, DOCKER_HOST, contexts or TLS settings.
        #[cfg(windows)]
        let docker =
            Docker::connect_with_named_pipe("//./pipe/docker_engine", 30, API_DEFAULT_VERSION);
        #[cfg(unix)]
        let docker = Docker::connect_with_unix("/var/run/docker.sock", 30, API_DEFAULT_VERSION);
        let docker = docker.map_err(map_error)?;
        let egress = Arc::new(DockerEgress::new(
            docker.clone(),
            egress_metadata,
            mutation_permitted.clone(),
        )?);
        Ok(Self {
            docker,
            metadata,
            policy: IsolationPolicy,
            timeout: OPERATION_TIMEOUT,
            mutation_permitted,
            egress,
        })
    }

    pub fn egress(&self) -> Arc<DockerEgress> {
        self.egress.clone()
    }

    /// Recheck the host's terminal supervisor state before every individual mutation.
    /// An already dispatched Docker request remains uncertain after guard loss.
    pub(super) fn require_mutation_permission(&self) -> Result<(), EnvironmentError> {
        if (self.mutation_permitted)() {
            Ok(())
        } else {
            Err(EnvironmentError::Unavailable)
        }
    }

    async fn bounded<T>(
        &self,
        operation: impl Future<Output = Result<T, EnvironmentError>>,
    ) -> Result<T, EnvironmentError> {
        tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| EnvironmentError::Timeout)?
    }

    async fn prepare(&self, env: &Environment) -> Result<(), EnvironmentError> {
        validate_record(env, self.metadata.installation_id())?;
        self.docker
            .clone()
            .negotiate_version()
            .await
            .map_err(map_error)?;
        Ok(())
    }

    pub(super) async fn volume(
        &self,
        env: &Environment,
    ) -> Result<VolumePresence, EnvironmentError> {
        let volume = match self.docker.inspect_volume(&env.volume_name).await {
            Ok(v) => v,
            Err(Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(VolumePresence::Missing),
            Err(e) => return Err(map_error(e)),
        };
        let value = json_value(volume)?;
        verify_labels(env, &value["Labels"])?;
        if value["Name"] != env.volume_name
            || value["Driver"] != "local"
            || !empty(&value["Options"])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        Ok(VolumePresence::Present)
    }

    pub(super) async fn container(
        &self,
        env: &Environment,
    ) -> Result<Option<Value>, EnvironmentError> {
        let value = match self
            .docker
            .inspect_container(&env.container_name, None::<InspectContainerOptions>)
            .await
        {
            Ok(v) => json_value(v)?,
            Err(Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(None),
            Err(e) => return Err(map_error(e)),
        };
        verify_labels(env, &value["Config"]["Labels"])?;
        if value["Name"] != format!("/{}", env.container_name)
            || value["Image"] != env.image_id
            || value["Config"]["Image"] != env.image_id
            || value["Id"]
                .as_str()
                .is_none_or(|id| id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(EnvironmentError::OwnershipConflict);
        }
        Ok(Some(value))
    }

    pub(super) async fn image(&self, env: &Environment) -> Result<(), EnvironmentError> {
        let value = json_value(
            self.docker
                .inspect_image(&env.image_id)
                .await
                .map_err(map_error)?,
        )?;
        if value["Id"] != env.image_id
            || value["Os"] != "linux"
            || value["Config"]["Labels"]["io.factory.environment.image"] != super::POLICY_VERSION
            || value["Config"]["User"] != "10000:10000"
            || !empty(&value["Config"]["Volumes"])
            || !empty(&value["Config"]["ExposedPorts"])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        Ok(())
    }

    /// Proxy liveness is separate from network ownership so a proxy outage never
    /// prevents an owned local shell. Only the verified dedicated bridge is accepted.
    pub(super) async fn verify_current_container(
        &self,
        env: &Environment,
        container: &Value,
    ) -> Result<(), EnvironmentError> {
        self.policy.verify_container(env, container)?;
        let network = self.egress.inspect_user_network(env).await?;
        let endpoint = &container["NetworkSettings"]["Networks"][&network.name];
        if endpoint["NetworkID"] != network.id
            && !(container["State"]["Running"] == false && endpoint["NetworkID"] == "")
        {
            return Err(EnvironmentError::OwnershipConflict);
        }
        Ok(())
    }

    async fn inspect_inner(
        &self,
        env: &Environment,
    ) -> Result<RuntimeInspection, EnvironmentError> {
        self.prepare(env).await?;
        let volume = self.volume(env).await?;
        if volume == VolumePresence::Missing
            && env.provisioning_state == ProvisioningState::VolumeReady
            && env.desired_state == DesiredState::Running
        {
            return Err(EnvironmentError::VolumeMissing);
        }
        let container = self.container(env).await?;
        let state = match container {
            None => ObservedState::NotCreated,
            Some(value) => {
                if env.desired_state == DesiredState::Running {
                    require_current_policy(env)?;
                    self.verify_current_container(env, &value).await?;
                    if volume == VolumePresence::Missing {
                        return Err(EnvironmentError::VolumeMissing);
                    }
                }
                match value["State"]["Status"].as_str() {
                    Some("running")
                        if value["State"]["Running"] == true
                            && value["State"]["Paused"] != true
                            && value["State"]["Restarting"] != true =>
                    {
                        ObservedState::Running
                    }
                    Some("created" | "exited") if value["State"]["Running"] == false => {
                        ObservedState::Stopped
                    }
                    _ => return Err(EnvironmentError::Unavailable),
                }
            }
        };
        Ok(RuntimeInspection { state, volume })
    }

    async fn start_inner(&self, env: &Environment) -> Result<(), EnvironmentError> {
        self.prepare(env).await?;
        require_current_policy(env)?;
        if self.volume(env).await? == VolumePresence::Missing {
            return Err(EnvironmentError::VolumeMissing);
        }
        self.image(env).await?;
        if self.container(env).await?.is_none() {
            self.require_mutation_permission()?;
            self.egress.prepare_user_network(env).await?;
            self.require_mutation_permission()?;
            let result = self
                .docker
                .create_container(
                    Some(CreateContainerOptions {
                        name: Some(env.container_name.clone()),
                        ..Default::default()
                    }),
                    self.policy.create_body(env),
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
        // Re-inspect even after successful creation or a racing name conflict.
        let container = self
            .container(env)
            .await?
            .ok_or(EnvironmentError::Unavailable)?;
        self.verify_current_container(env, &container).await?;
        if container["State"]["Running"] == true {
            return Ok(());
        }
        self.require_mutation_permission()?;
        self.egress.prepare_user_network(env).await?;
        let id = container["Id"]
            .as_str()
            .ok_or(EnvironmentError::OwnershipConflict)?;
        self.require_mutation_permission()?;
        self.docker
            .start_container(id, None::<StartContainerOptions>)
            .await
            .map_err(map_error)
    }
}

#[async_trait]
impl EnvironmentRuntime for DockerEnvironmentRuntime {
    async fn inspect(&self, env: &Environment) -> Result<RuntimeInspection, EnvironmentError> {
        self.bounded(self.inspect_inner(env)).await
    }
    async fn create_volume(&self, env: &Environment) -> Result<(), EnvironmentError> {
        self.bounded(async {
            self.prepare(env).await?;
            require_current_policy(env)?;
            if self.volume(env).await? == VolumePresence::Present {
                return Ok(());
            }
            if env.provisioning_state != ProvisioningState::VolumeCreating {
                return Err(EnvironmentError::ProvisioningUncertain);
            }
            self.require_mutation_permission()?;
            self.docker
                .create_volume(VolumeCreateRequest {
                    name: Some(env.volume_name.clone()),
                    driver: Some("local".into()),
                    labels: Some(labels(env)),
                    ..Default::default()
                })
                .await
                .map_err(map_error)?;
            if self.volume(env).await? != VolumePresence::Present {
                return Err(EnvironmentError::Unavailable);
            }
            Ok(())
        })
        .await
    }
    async fn start(&self, env: &Environment) -> Result<(), EnvironmentError> {
        self.bounded(self.start_inner(env)).await
    }
    async fn stop(&self, env: &Environment) -> Result<(), EnvironmentError> {
        self.bounded(async {
            self.prepare(env).await?;
            // Stop remains possible for a proved owned container even if image/policy changed
            // or its data volume vanished. Never mutate a resource selected only by name.
            let Some(container) = self.container(env).await? else {
                return Ok(());
            };
            if container["State"]["Running"] == false
                && matches!(
                    container["State"]["Status"].as_str(),
                    Some("created" | "exited")
                )
            {
                return Ok(());
            }
            let id = container["Id"]
                .as_str()
                .ok_or(EnvironmentError::OwnershipConflict)?;
            self.require_mutation_permission()?;
            self.docker
                .stop_container(
                    id,
                    Some(StopContainerOptions {
                        t: Some(10),
                        ..Default::default()
                    }),
                )
                .await
                .map_err(map_error)
        })
        .await
    }
}

fn json_value(value: impl Serialize) -> Result<Value, EnvironmentError> {
    serde_json::to_value(value).map_err(|_| EnvironmentError::Unavailable)
}
fn map_error(error: Error) -> EnvironmentError {
    match error {
        Error::RequestTimeoutError => EnvironmentError::Timeout,
        Error::DockerResponseServerError {
            status_code: 409, ..
        } => EnvironmentError::Conflict,
        _ => EnvironmentError::Unavailable,
    }
}
