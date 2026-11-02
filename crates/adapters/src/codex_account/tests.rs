use super::*;
use crate::environments::{POLICY_VERSION, PreparationMetadata};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
};
use factory_core::environments::*;
use futures_util::{FutureExt, StreamExt};

fn observation(snapshot: ConnectionSnapshot) -> Pending {
    let (_, state) = watch::channel(snapshot);
    let (cancel, _) = oneshot::channel();
    let (_, finished) = oneshot::channel();
    Pending {
        operation_id: Uuid::new_v4(),
        state,
        cancel: Some(cancel),
        finished,
    }
}

#[test]
fn only_connecting_observations_survive_repeated_reads() {
    for error in [
        AgentConnectionError::CleanupUncertain,
        AgentConnectionError::Cancelled,
        AgentConnectionError::Expired,
        AgentConnectionError::LoginFailed,
    ] {
        let mut pending = Some(observation(ConnectionSnapshot::failure(error, Utc::now())));
        assert_eq!(take_observation(&mut pending).unwrap().error, Some(error));
        assert!(take_observation(&mut pending).is_none());
    }
    let mut snapshot = ConnectionSnapshot::account(None, Utc::now());
    snapshot.status = ConnectionStatus::Connecting;
    snapshot.challenge = Some(LoginChallenge {
        operation_id: Uuid::new_v4(),
        verification_url: "https://auth.openai.com/codex/device".into(),
        user_code: Some("FIXTURE-ONLY".into()),
        expires_at: Utc::now() + chrono::Duration::minutes(1),
    });
    let mut pending = Some(observation(snapshot));
    for _ in 0..2 {
        assert_eq!(
            take_observation(&mut pending)
                .unwrap()
                .challenge
                .unwrap()
                .user_code
                .as_deref(),
            Some("FIXTURE-ONLY")
        );
        assert!(pending.is_some());
    }
}

async fn recovered_controller_can_be_read(
    adapter: &CodexAccountAdapter,
    env: &Environment,
) -> Result<(), Box<dyn std::error::Error>> {
    // Hold the real fixed controller lock without starting OAuth. An uncertain
    // observation must not invent Disconnected while this process still exists.
    let (id, mut controller) = adapter.open(env).await?;
    controller.initialize().await?;
    let slot = adapter.slot(env.id)?;
    *slot.lock().await = Some(observation(ConnectionSnapshot::failure(
        AgentConnectionError::CleanupUncertain,
        Utc::now(),
    )));
    assert_eq!(
        adapter.read(env).await?.error,
        Some(AgentConnectionError::CleanupUncertain)
    );
    let blocked = adapter.read(env).await;
    close(&adapter.runtime, &id, controller).await?;
    assert!(matches!(blocked, Err(AgentConnectionError::Conflict)));
    assert_eq!(
        adapter.read(env).await?.status,
        ConnectionStatus::Disconnected
    );
    Ok(())
}

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
    let egress = crate::environments::fixture::egress_metadata(metadata.installation_id());
    let runtime =
        Arc::new(DockerEnvironmentRuntime::connect(metadata, egress, Arc::new(|| true)).unwrap());
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
        recovered_controller_can_be_read(&adapter,&env).await?;
        let container=docker.inspect_container(&env.container_name,None).await?;
        let id=container.id.unwrap();
        seed(&docker,&id).await?;
        assert!(matches!(adapter.read(&env).await,Err(AgentConnectionError::UnsupportedMethod)));
        let slot = adapter.slot(env.id)?;
        *slot.lock().await = Some(observation(ConnectionSnapshot::failure(AgentConnectionError::CleanupUncertain, Utc::now())));
        assert_eq!(adapter.read(&env).await?.error, Some(AgentConnectionError::CleanupUncertain));
        runtime.stop(&env).await?;runtime.start(&env).await?;
        assert!(matches!(adapter.read(&env).await,Err(AgentConnectionError::UnsupportedMethod)));
        crate::environments::fixture::stop_proxy(&runtime).await;
        assert!(matches!(adapter.read(&env).await, Err(AgentConnectionError::Unavailable)));
        assert!(matches!(adapter.begin_login(&env, Uuid::new_v4(), Utc::now()+chrono::Duration::minutes(5)).await, Err(AgentConnectionError::Unavailable)));
        assert_eq!(adapter.logout(&env).await?.status,ConnectionStatus::Disconnected);
        assert_eq!(runtime.inspect(&env).await?.state, ObservedState::Running);
        let exec=docker.create_exec(&id,CreateExecOptions{cmd:Some(vec!["/bin/sh","-c","test -f /home/factory/Documents/witness && test ! -f /home/factory/.codex/auth.json && test $(stat -c %a /home/factory/.codex) = 700"]),user:Some("10000:10000"),..Default::default()}).await?;
        docker.start_exec(&exec.id,Some(StartExecOptions{detach:true,tty:false,output_capacity:None})).await?;
        assert_eq!(runtime.wait_process_exit(&exec.id,Duration::from_secs(5)).await?,0);
        Ok::<(),Box<dyn std::error::Error>>(())
    }).catch_unwind().await;
    crate::environments::fixture::cleanup(&runtime, &[&env]).await;
    result.unwrap().unwrap();
}
