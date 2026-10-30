use super::*;
use crate::environments::{POLICY_VERSION, PreparationMetadata};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    container::LogOutput,
    query_parameters::{RemoveContainerOptions, RemoveVolumeOptions},
};
use chrono::Utc;
use factory_core::environments::*;
use serde_json::{Value, json};
use uuid::Uuid;

#[tokio::test]
async fn docker_output_is_chunked_and_rejects_oversized_daemon_buffers() {
    let bytes = vec![b'x'; OUTPUT_CHUNK_BYTES + 7];
    let mut output = Output {
        stream: Box::pin(futures_util::stream::iter([Ok(LogOutput::StdOut {
            message: bytes.into(),
        })])),
        pending: vec![],
        offset: 0,
    };
    assert_eq!(
        output.read().await.unwrap().unwrap().len(),
        OUTPUT_CHUNK_BYTES
    );
    assert_eq!(output.read().await.unwrap().unwrap().len(), 7);
    assert!(output.read().await.unwrap().is_none());
    let mut oversized = Output {
        stream: Box::pin(futures_util::stream::iter([Ok(LogOutput::StdOut {
            message: vec![0; MAX_OUTPUT_BUFFER_BYTES + 1].into(),
        })])),
        pending: vec![],
        offset: 0,
    };
    assert_eq!(oversized.read().await, Err(TerminalError::Unavailable));
}

async fn marker(
    output: &mut dyn TerminalOutput,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(chunk) = output.read().await? {
            text.push_str(&String::from_utf8_lossy(&chunk));
            if text.contains(expected) {
                return Ok(());
            }
            if text.len() > 65536 {
                return Err("Fixture output exceeded its bound".into());
            }
        }
        Err(format!("Missing fixture marker: {expected}").into())
    })
    .await
    .map_err(|_| format!("Timed out waiting for fixture marker: {expected}"))?
}

async fn exercise(
    runtime: Arc<DockerEnvironmentRuntime>,
    environment: &Environment,
) -> Result<(), Box<dyn std::error::Error>> {
    let terminal = DockerTerminalRuntime::new(runtime.clone());
    let mut first = terminal.attach(environment).await?;
    first
        .input
        .write(b"printf 'attached-%s\\n' ready\r")
        .await?;
    marker(first.output.as_mut(), "attached-ready").await?;
    terminal
        .resize(&first.handle, TerminalSize::new(132, 41)?)
        .await?;
    first.input.write(b"stty size\r").await?;
    marker(first.output.as_mut(), "41 132").await?;
    first
        .input
        .write(b"printf 'flood-%s\\n' started; sleep 1; head -c 4194304 /dev/zero | tr '\\000' x; printf '\\n'; printf preserved > ~/Documents/terminal-witness\r")
        .await?;
    marker(first.output.as_mut(), "flood-started").await?;
    let handle = first.handle.clone();
    drop(first);
    terminal.detach(environment, &handle).await?;
    // tmux keeps consuming the 4-MiB output with no browser/API reader attached.
    // A new adapter has no in-memory terminal state, as after restarting the API.
    let restarted = DockerTerminalRuntime::new(runtime.clone());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut second = restarted.attach(environment).await?;
    second
        .input
        .write(b"printf '%s-reattached\\n' \"$(cat ~/Documents/terminal-witness)\"\r")
        .await?;
    marker(second.output.as_mut(), "preserved-reattached").await?;
    let handle = second.handle.clone();
    drop(second);
    restarted.detach(environment, &handle).await?;
    runtime.stop(environment).await?;
    runtime.start(environment).await?;
    let mut fresh = restarted.attach(environment).await?;
    fresh
        .input
        .write(b"printf '%s-restart\\n' \"$(cat ~/Documents/terminal-witness)\"\r")
        .await?;
    marker(fresh.output.as_mut(), "preserved-restart").await?;
    let handle = fresh.handle.clone();
    drop(fresh);
    restarted.detach(environment, &handle).await?;
    Ok(())
}

fn verify_labels(environment: &Environment, labels: &Value) {
    for (name, expected) in [
        (
            "io.factory.installation",
            environment.installation_id.to_string(),
        ),
        ("io.factory.environment", environment.id.to_string()),
        ("io.factory.user", environment.user_id.to_string()),
        ("io.factory.policy", environment.policy_version.clone()),
        ("io.factory.image", environment.image_id.clone()),
    ] {
        assert_eq!(labels[name], expected);
    }
}

#[tokio::test]
#[ignore = "Requires explicit FACTORY_PROCESS_TEST_IMAGE and the local Docker daemon; creates UUID-labelled disposable resources"]
async fn local_docker_terminal_adapter_reconnects_without_losing_work() {
    let image =
        std::env::var("FACTORY_PROCESS_TEST_IMAGE").expect("Explicit immutable fixture image");
    let installation_id = Uuid::new_v4();
    let id = Uuid::new_v4();
    let name = format!("factory-{}-{}", installation_id.simple(), id.simple());
    let metadata: PreparationMetadata = serde_json::from_value(json!({"schemaVersion":1,"installationId":installation_id.to_string(),"imageId":image,"policyVersion":POLICY_VERSION})).unwrap();
    let mut environment = Environment {
        id,
        user_id: Uuid::new_v4(),
        installation_id,
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
    runtime.create_volume(&environment).await.unwrap();
    environment.provisioning_state = ProvisioningState::VolumeReady;
    let result = async {
        runtime.start(&environment).await?;
        exercise(runtime.clone(), &environment).await
    }
    .await;
    runtime.stop(&environment).await.unwrap();
    #[cfg(windows)]
    let docker =
        Docker::connect_with_named_pipe("//./pipe/docker_engine", 30, API_DEFAULT_VERSION).unwrap();
    #[cfg(unix)]
    let docker =
        Docker::connect_with_unix("/var/run/docker.sock", 30, API_DEFAULT_VERSION).unwrap();
    let container = serde_json::to_value(
        docker
            .inspect_container(&environment.container_name, None)
            .await
            .unwrap(),
    )
    .unwrap();
    verify_labels(&environment, &container["Config"]["Labels"]);
    assert_eq!(container["State"]["Running"], false);
    assert_eq!(container["Image"], environment.image_id);
    docker
        .remove_container(
            container["Id"].as_str().unwrap(),
            None::<RemoveContainerOptions>,
        )
        .await
        .unwrap();
    let volume = serde_json::to_value(
        docker
            .inspect_volume(&environment.volume_name)
            .await
            .unwrap(),
    )
    .unwrap();
    verify_labels(&environment, &volume["Labels"]);
    assert_eq!(volume["Name"], environment.volume_name);
    docker
        .remove_volume(&environment.volume_name, None::<RemoveVolumeOptions>)
        .await
        .unwrap();
    result.unwrap();
}
