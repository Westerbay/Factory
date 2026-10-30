use super::*;
use crate::environments::{POLICY_VERSION, PreparationMetadata};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    query_parameters::{RemoveContainerOptions, RemoveVolumeOptions},
};
use factory_core::environments::*;
use futures_util::{FutureExt, StreamExt};

async fn seed(docker: &Docker, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Fixed synthetic credential; no environment, host configuration or real account is read.
    let exec=docker.create_exec(id,CreateExecOptions{
        cmd:Some(vec!["/bin/sh","-c",r#"umask 077; mkdir -p /home/factory/.codex; printf '%s' '{"OPENAI_API_KEY":"synthetic-fixture-not-a-real-key"}' > /home/factory/.codex/auth.json; printf 'keep' > /home/factory/Documents/witness"#]),
        user:Some("10000:10000"),attach_stdout:Some(true),attach_stderr:Some(true),..Default::default()
    }).await?;
    if let StartExecResults::Attached { mut output, .. } = docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                tty: false,
                output_capacity: Some(4096),
            }),
        )
        .await?
    {
        while let Some(chunk) = output.next().await {
            chunk?;
        }
    }
    assert_eq!(docker.inspect_exec(&exec.id).await?.exit_code, Some(0));
    Ok(())
}

#[tokio::test]
#[ignore = "Build the image and set FACTORY_PROCESS_TEST_IMAGE to its immutable ID; local Docker only"]
async fn local_docker_synthetic_cache_read_logout_and_persistence() {
    let image =
        std::env::var("FACTORY_PROCESS_TEST_IMAGE").expect("Explicit fixture image required");
    let installation = Uuid::new_v4();
    let environment_id = Uuid::new_v4();
    let name = format!(
        "factory-{}-{}",
        installation.simple(),
        environment_id.simple()
    );
    let metadata:PreparationMetadata=serde_json::from_value(json!({"schemaVersion":1,"installationId":installation.to_string(),"imageId":image,"policyVersion":POLICY_VERSION})).unwrap();
    let mut env = Environment {
        id: environment_id,
        user_id: Uuid::new_v4(),
        installation_id: installation,
        desired_state: DesiredState::Running,
        observed_state: ObservedState::NotCreated,
        generation: 1,
        provisioning_state: ProvisioningState::VolumeCreating,
        volume_name: format!("{name}-home"),
        container_name: name,
        image_id: image,
        policy_version: POLICY_VERSION.into(),
        observed_at: None,
        error_code: None,
        next_reconcile_at: Utc::now(),
    };
    let runtime = Arc::new(DockerEnvironmentRuntime::connect(metadata, Arc::new(|| true)).unwrap());
    #[cfg(windows)]
    let docker =
        Docker::connect_with_named_pipe("//./pipe/docker_engine", 30, API_DEFAULT_VERSION).unwrap();
    #[cfg(unix)]
    let docker =
        Docker::connect_with_unix("/var/run/docker.sock", 30, API_DEFAULT_VERSION).unwrap();
    let adapter = CodexAccountAdapter::new(runtime.clone());
    let result=std::panic::AssertUnwindSafe(async {
        runtime.create_volume(&env).await?;env.provisioning_state=ProvisioningState::VolumeReady;
        runtime.start(&env).await?;
        assert_eq!(adapter.read(&env).await?.status,ConnectionStatus::Disconnected);
        let container=docker.inspect_container(&env.container_name,None).await?;
        let id=container.id.unwrap();
        seed(&docker,&id).await?;
        assert!(matches!(adapter.read(&env).await,Err(AgentConnectionError::UnsupportedMethod)));
        runtime.stop(&env).await?;runtime.start(&env).await?;
        assert!(matches!(adapter.read(&env).await,Err(AgentConnectionError::UnsupportedMethod)));
        assert_eq!(adapter.logout(&env).await?.status,ConnectionStatus::Disconnected);
        assert_eq!(adapter.read(&env).await?.status,ConnectionStatus::Disconnected);
        let exec=docker.create_exec(&id,CreateExecOptions{cmd:Some(vec!["/bin/sh","-c","test -f /home/factory/Documents/witness && test ! -f /home/factory/.codex/auth.json && test $(stat -c %a /home/factory/.codex) = 700"]),user:Some("10000:10000"),..Default::default()}).await?;
        docker.start_exec(&exec.id,Some(StartExecOptions{detach:true,tty:false,output_capacity:None})).await?;
        assert_eq!(runtime.wait_process_exit(&exec.id,Duration::from_secs(5)).await?,0);
        Ok::<(),Box<dyn std::error::Error>>(())
    }).catch_unwind().await;
    // Ownership labels and immutable IDs guard cleanup even if an assertion route failed.
    runtime.stop(&env).await.unwrap();
    let container = docker
        .inspect_container(&env.container_name, None)
        .await
        .unwrap();
    let labels = container.config.unwrap().labels.unwrap();
    assert_eq!(
        labels.get("io.factory.installation"),
        Some(&installation.to_string())
    );
    assert_eq!(
        labels.get("io.factory.environment"),
        Some(&environment_id.to_string())
    );
    docker
        .remove_container(&container.id.unwrap(), None::<RemoveContainerOptions>)
        .await
        .unwrap();
    let volume = docker.inspect_volume(&env.volume_name).await.unwrap();
    assert_eq!(
        volume.labels.get("io.factory.environment"),
        Some(&environment_id.to_string())
    );
    docker
        .remove_volume(&env.volume_name, None::<RemoveVolumeOptions>)
        .await
        .unwrap();
    result.unwrap().unwrap();
}
