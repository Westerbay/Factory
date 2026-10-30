use super::{
    DockerEnvironmentRuntime, PreparationMetadata,
    policy::{empty, validate_record},
};
use crate::egress::{DockerEgress, EgressMetadata};
use bollard::{
    errors::Error,
    query_parameters::{InspectContainerOptions, RemoveContainerOptions},
};
use factory_core::environments::*;
use serde_json::Value;
use std::{future::Future, sync::Arc};

/// Explicit operator replacement. Never used by normal Start or reconciliation.
pub struct DockerEnvironmentUpgradeRuntime {
    runtime: DockerEnvironmentRuntime,
    egress: DockerEgress,
}

impl DockerEnvironmentUpgradeRuntime {
    pub fn connect(
        metadata: PreparationMetadata,
        egress: EgressMetadata,
        permitted: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, EnvironmentError> {
        if metadata.installation_id() != egress.installation_id {
            return Err(EnvironmentError::OwnershipConflict);
        }
        let runtime = DockerEnvironmentRuntime::connect(metadata, permitted.clone())?;
        let egress = DockerEgress::new(runtime.docker.clone(), egress, permitted)?;
        Ok(Self { runtime, egress })
    }

    async fn bounded<T>(
        &self,
        operation: impl Future<Output = Result<T, EnvironmentError>>,
    ) -> Result<T, EnvironmentError> {
        tokio::time::timeout(self.runtime.timeout, operation)
            .await
            .map_err(|_| EnvironmentError::Timeout)?
    }

    async fn inspect(&self, env: &Environment) -> Result<Option<String>, EnvironmentError> {
        validate_record(env, self.runtime.metadata.installation_id())?;
        if env.desired_state != DesiredState::Stopped {
            return Err(EnvironmentError::Conflict);
        }
        let volume = self
            .runtime
            .docker
            .inspect_volume(&env.volume_name)
            .await
            .map_err(|error| {
                if missing(&error) {
                    EnvironmentError::VolumeMissing
                } else {
                    EnvironmentError::Unavailable
                }
            })?;
        let volume = serde_json::to_value(volume).map_err(|_| EnvironmentError::Unavailable)?;
        // Home ownership is stable across image generations. Legacy image/policy labels
        // describe creation, not its current container; never rewrite or replace the home.
        for (key, value) in [
            ("io.factory.installation", env.installation_id),
            ("io.factory.environment", env.id),
            ("io.factory.user", env.user_id),
        ] {
            if volume["Labels"][key] != value.to_string() {
                return Err(EnvironmentError::OwnershipConflict);
            }
        }
        if volume["Name"] != env.volume_name
            || volume["Driver"] != "local"
            || !empty(&volume["Options"])
        {
            return Err(EnvironmentError::PolicyMismatch);
        }
        let Some(container) = self.runtime.container(env).await? else {
            return Ok(None);
        };
        require_stopped_home(env, &container)?;
        Ok(Some(
            container["Id"]
                .as_str()
                .ok_or(EnvironmentError::OwnershipConflict)?
                .to_owned(),
        ))
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

fn require_stopped_home(env: &Environment, container: &Value) -> Result<(), EnvironmentError> {
    if container["State"]["Running"] != false
        || container["State"]["Paused"] != false
        || container["State"]["Restarting"] != false
        || !matches!(
            container["State"]["Status"].as_str(),
            Some("created" | "exited")
        )
    {
        return Err(EnvironmentError::Conflict);
    }
    let mounts = container["Mounts"]
        .as_array()
        .ok_or(EnvironmentError::PolicyMismatch)?;
    if mounts.len() != 1
        || mounts[0]["Type"] != "volume"
        || mounts[0]["Name"] != env.volume_name
        || mounts[0]["Destination"] != "/home/factory"
        || mounts[0]["Driver"] != "local"
        || mounts[0]["RW"] != true
    {
        return Err(EnvironmentError::PolicyMismatch);
    }
    Ok(())
}

#[async_trait::async_trait]
impl EnvironmentUpgradeRuntime for DockerEnvironmentUpgradeRuntime {
    async fn inspect_upgrade(&self, env: &Environment) -> Result<Option<String>, EnvironmentError> {
        self.bounded(self.inspect(env)).await
    }

    async fn prepare_upgrade(&self, target: &Environment) -> Result<(), EnvironmentError> {
        self.bounded(async {
            validate_record(target, self.runtime.metadata.installation_id())?;
            if target.image_id != self.runtime.metadata.image_id()
                || target.policy_version != self.runtime.metadata.policy_version()
            {
                return Err(EnvironmentError::PolicyMismatch);
            }
            self.runtime.image(target).await?;
            self.runtime.require_mutation_permission()?;
            self.egress.prepare_user_network(target).await?;
            Ok(())
        })
        .await
    }

    async fn remove_upgrade_container(
        &self,
        env: &Environment,
        id: &str,
    ) -> Result<(), EnvironmentError> {
        self.bounded(async {
            if id.len() != 64
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(EnvironmentError::InvalidConfiguration);
            }
            match self.inspect(env).await? {
                Some(actual) if actual != id => return Err(EnvironmentError::OwnershipConflict),
                Some(_) => {}
                None => {
                    // A renamed old container is a conflict, not a completed removal.
                    return match self
                        .runtime
                        .docker
                        .inspect_container(id, None::<InspectContainerOptions>)
                        .await
                    {
                        Err(error) if missing(&error) => Ok(()),
                        Err(_) => Err(EnvironmentError::Unavailable),
                        Ok(_) => Err(EnvironmentError::OwnershipConflict),
                    };
                }
            }
            self.runtime.require_mutation_permission()?;
            // Docker refuses a concurrent start because force=false. Never pass a name
            // or v=true, so replacement races cannot delete another container or home.
            match self
                .runtime
                .docker
                .remove_container(
                    id,
                    Some(RemoveContainerOptions {
                        force: false,
                        v: false,
                        link: false,
                    }),
                )
                .await
            {
                Ok(()) => Ok(()),
                Err(error) if missing(&error) => Ok(()),
                Err(Error::DockerResponseServerError {
                    status_code: 409, ..
                }) => Err(EnvironmentError::Conflict),
                Err(_) => Err(EnvironmentError::Unavailable),
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::{
        exec::{CreateExecOptions, StartExecResults},
        query_parameters::*,
    };
    use futures_util::{FutureExt, StreamExt};
    use serde_json::json;
    use uuid::Uuid;

    async fn command(docker: &bollard::Docker, id: &str, script: &str) -> String {
        let exec = docker
            .create_exec(
                id,
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(vec!["/bin/bash", "-c", script]),
                    user: Some("10000:10000"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut result = String::new();
        if let StartExecResults::Attached { mut output, .. } =
            docker.start_exec(&exec.id, None).await.unwrap()
        {
            while let Some(frame) =
                tokio::time::timeout(std::time::Duration::from_secs(10), output.next())
                    .await
                    .unwrap()
            {
                result.push_str(&frame.unwrap().to_string());
                assert!(result.len() < 4096);
            }
        }
        assert_eq!(
            docker.inspect_exec(&exec.id).await.unwrap().exit_code,
            Some(0)
        );
        result
    }

    #[tokio::test]
    #[ignore = "Set FACTORY_PROCESS_TEST_IMAGE to the explicitly built immutable fixture image; local Docker only"]
    async fn local_docker_upgrade_removes_only_stopped_id_and_preserves_home() {
        let image = std::env::var("FACTORY_PROCESS_TEST_IMAGE").expect("explicit fixture image");
        let installation = Uuid::new_v4();
        let id = Uuid::new_v4();
        let name = format!("factory-{}-{}", installation.simple(), id.simple());
        let metadata: PreparationMetadata = serde_json::from_value(json!({"schemaVersion":1,"installationId":installation.to_string(),"imageId":image,"policyVersion":super::super::POLICY_VERSION})).unwrap();
        let env = Environment {
            id,
            user_id: Uuid::new_v4(),
            installation_id: installation,
            desired_state: DesiredState::Stopped,
            observed_state: ObservedState::Stopped,
            generation: 2,
            provisioning_state: ProvisioningState::VolumeReady,
            volume_name: format!("{name}-home"),
            container_name: name,
            image_id: image.clone(),
            policy_version: "1".into(),
            observed_at: None,
            error_code: None,
            next_reconcile_at: chrono::Utc::now(),
        };
        // Egress is not mutated by the removal test. Its real preparation/isolation has
        // a separate guarded fixture; the constructor still receives valid metadata.
        let egress = EgressMetadata {
            schema_version: 1,
            installation_id: installation,
            image_id: image,
            host_ipv4: vec![std::net::Ipv4Addr::LOCALHOST],
        };
        let runtime =
            DockerEnvironmentUpgradeRuntime::connect(metadata, egress, Arc::new(|| true)).unwrap();
        let docker = &runtime.runtime.docker;
        assert!(
            matches!(docker.inspect_volume(&env.volume_name).await, Err(error) if missing(&error))
        );
        assert!(
            matches!(docker.inspect_container(&env.container_name, None::<InspectContainerOptions>).await, Err(error) if missing(&error))
        );
        let mut created_ids = Vec::new();
        let result = std::panic::AssertUnwindSafe(async {
            assert_eq!(
                runtime.inspect_upgrade(&env).await,
                Err(EnvironmentError::VolumeMissing)
            );
            docker
                .create_volume(bollard::models::VolumeCreateRequest {
                    name: Some(env.volume_name.clone()),
                    driver: Some("local".into()),
                    labels: Some(super::super::policy::labels(&env)),
                    ..Default::default()
                })
                .await
                .unwrap();
            let body = runtime.runtime.policy.create_body(&env);
            let old = docker
                .create_container(
                    Some(CreateContainerOptions {
                        name: Some(env.container_name.clone()),
                        ..Default::default()
                    }),
                    body,
                )
                .await
                .unwrap()
                .id;
            created_ids.push(old.clone());
            docker
                .start_container(&old, None::<StartContainerOptions>)
                .await
                .unwrap();
            command(
                docker,
                &old,
                "printf 'upgrade-home-witness' > /home/factory/Documents/upgrade-witness",
            )
            .await;
            assert_eq!(
                runtime.inspect_upgrade(&env).await,
                Err(EnvironmentError::Conflict)
            );
            assert_eq!(
                runtime.remove_upgrade_container(&env, &old).await,
                Err(EnvironmentError::Conflict)
            );
            docker
                .stop_container(
                    &old,
                    Some(StopContainerOptions {
                        t: Some(1),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
            assert_eq!(
                runtime.inspect_upgrade(&env).await.unwrap(),
                Some(old.clone())
            );
            assert_eq!(
                runtime
                    .remove_upgrade_container(&env, &"f".repeat(64))
                    .await,
                Err(EnvironmentError::OwnershipConflict)
            );
            docker
                .rename_container(
                    &old,
                    RenameContainerOptions {
                        name: format!("{}-renamed", env.container_name),
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                runtime.remove_upgrade_container(&env, &old).await,
                Err(EnvironmentError::OwnershipConflict)
            );
            docker
                .rename_container(
                    &old,
                    RenameContainerOptions {
                        name: env.container_name.clone(),
                    },
                )
                .await
                .unwrap();
            let mut foreign = env.clone();
            foreign.user_id = Uuid::new_v4();
            assert_eq!(
                runtime.inspect_upgrade(&foreign).await,
                Err(EnvironmentError::OwnershipConflict)
            );
            runtime.remove_upgrade_container(&env, &old).await.unwrap();
            assert_eq!(runtime.inspect_upgrade(&env).await.unwrap(), None);
            runtime.remove_upgrade_container(&env, &old).await.unwrap();
            assert!(docker.inspect_volume(&env.volume_name).await.is_ok());
            let mut target = env.clone();
            target.policy_version = super::super::POLICY_VERSION.into();
            runtime.runtime.image(&target).await.unwrap();
            let replacement = docker
                .create_container(
                    Some(CreateContainerOptions {
                        name: Some(env.container_name.clone()),
                        ..Default::default()
                    }),
                    runtime.runtime.policy.create_body(&target),
                )
                .await
                .unwrap()
                .id;
            created_ids.push(replacement.clone());
            assert_ne!(replacement, old);
            // Old-operation retry refuses the replacement, even while it is stopped.
            assert_eq!(
                runtime.remove_upgrade_container(&env, &old).await,
                Err(EnvironmentError::OwnershipConflict)
            );
            assert_eq!(
                runtime.inspect_upgrade(&target).await.unwrap(),
                Some(replacement.clone()),
                "legacy volume labels remain valid stable ownership"
            );
            docker
                .start_container(&replacement, None::<StartContainerOptions>)
                .await
                .unwrap();
            assert_eq!(
                command(
                    docker,
                    &replacement,
                    "cat /home/factory/Documents/upgrade-witness"
                )
                .await,
                "upgrade-home-witness"
            );
        })
        .catch_unwind()
        .await;
        // Cleanup only IDs created by this fixture, after rechecking immutable ownership.
        for id in created_ids {
            match docker
                .inspect_container(&id, None::<InspectContainerOptions>)
                .await
            {
                Err(error) if missing(&error) => {}
                Err(error) => panic!("fixture cleanup inspection failed: {error}"),
                Ok(container) => {
                    let value = serde_json::to_value(container).unwrap();
                    assert_eq!(value["Id"], id);
                    assert_eq!(
                        value["Config"]["Labels"]["io.factory.installation"],
                        installation.to_string()
                    );
                    assert_eq!(
                        value["Config"]["Labels"]["io.factory.environment"],
                        env.id.to_string()
                    );
                    if value["State"]["Running"] == true {
                        docker
                            .stop_container(
                                &id,
                                Some(StopContainerOptions {
                                    t: Some(1),
                                    ..Default::default()
                                }),
                            )
                            .await
                            .unwrap();
                    }
                    docker
                        .remove_container(&id, None::<RemoveContainerOptions>)
                        .await
                        .unwrap();
                }
            }
        }
        let volume = match docker.inspect_volume(&env.volume_name).await {
            Ok(volume) => Some(volume),
            Err(error) if missing(&error) => None,
            Err(error) => panic!("fixture volume cleanup inspection failed: {error}"),
        };
        if let Some(volume) = volume {
            let value = serde_json::to_value(volume).unwrap();
            assert_eq!(
                value["Labels"]["io.factory.installation"],
                installation.to_string()
            );
            assert_eq!(
                value["Labels"]["io.factory.environment"],
                env.id.to_string()
            );
            docker
                .remove_volume(&env.volume_name, None::<RemoveVolumeOptions>)
                .await
                .unwrap();
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
