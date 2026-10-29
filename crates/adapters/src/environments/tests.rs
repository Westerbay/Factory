use super::*;
use bollard::{API_DEFAULT_VERSION, Docker};
use chrono::Utc;
use factory_core::environments::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

fn metadata() -> PreparationMetadata {
    serde_json::from_value(json!({"schemaVersion":1,"installationId":Uuid::new_v4().to_string(),"imageId":format!("sha256:{}", "a".repeat(64)),"policyVersion":"2"})).unwrap()
}
fn environment(metadata: &PreparationMetadata) -> Environment {
    let id = Uuid::new_v4();
    let name = format!(
        "factory-{}-{}",
        metadata.installation_id().simple(),
        id.simple()
    );
    Environment {
        id,
        user_id: Uuid::new_v4(),
        installation_id: metadata.installation_id(),
        desired_state: DesiredState::Running,
        observed_state: ObservedState::NotCreated,
        generation: 1,
        provisioning_state: ProvisioningState::VolumeCreating,
        volume_name: format!("{name}-home"),
        container_name: name,
        image_id: metadata.image_id().into(),
        policy_version: "2".into(),
        observed_at: None,
        error_code: None,
        next_reconcile_at: Utc::now(),
    }
}
fn policy() -> IsolationPolicy {
    IsolationPolicy
}
fn volume(env: &Environment) -> Value {
    json!({"Name":env.volume_name,"Driver":"local","Mountpoint":"/fixture/home","Labels":policy::labels(env),"Scope":"local","Options":{}})
}
fn container(env: &Environment) -> Value {
    let body = serde_json::to_value(policy().create_body(env)).unwrap();
    json!({"Id":"b".repeat(64),"Name":format!("/{}",env.container_name),"Image":env.image_id,
        "Config":body,"HostConfig":body["HostConfig"],"State":{"Status":"running","Running":true},
        "Mounts":[{"Type":"volume","Name":env.volume_name,"Destination":"/home/factory","RW":true,"Driver":"local"}]})
}

type Reply = (
    &'static str,
    String,
    u16,
    Value,
    Duration,
    Option<Arc<AtomicBool>>,
);
struct Fixture {
    runtime: DockerEnvironmentRuntime,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    task: tokio::task::JoinHandle<()>,
    requests: Arc<AtomicUsize>,
}
#[derive(Clone)]
struct FixtureState {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<AtomicUsize>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture(metadata: PreparationMetadata, replies: Vec<Reply>, timeout: Duration) -> Fixture {
    use axum::{
        Router,
        body::Body,
        extract::State,
        http::{Request, Response},
        routing::any,
    };
    let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
    let requests = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .fallback(any(
            |State(state): State<FixtureState>, request: Request<Body>| async move {
                state.requests.fetch_add(1, Ordering::SeqCst);
                let next = state
                    .replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected Docker request");
                assert_eq!(request.method().as_str(), next.0);
                assert!(
                    request.uri().path().ends_with(&next.1),
                    "unexpected Docker path {}",
                    request.uri()
                );
                tokio::time::sleep(next.4).await;
                if let Some(permission) = next.5 {
                    permission.store(false, Ordering::SeqCst);
                }
                Response::builder()
                    .status(next.2)
                    .header("content-type", "application/json")
                    .body(Body::from(next.3.to_string()))
                    .unwrap()
            },
        ))
        .with_state(FixtureState {
            replies: replies.clone(),
            requests: requests.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    // HTTP exists only in this private test constructor; production exposes local transport only.
    let docker =
        Docker::connect_with_http(&format!("http://{address}"), 5, API_DEFAULT_VERSION).unwrap();
    Fixture {
        runtime: DockerEnvironmentRuntime {
            docker,
            metadata,
            policy: policy(),
            timeout,
            mutation_permitted: Arc::new(|| true),
        },
        replies,
        task,
        requests,
    }
}
fn reply(method: &'static str, path: String, status: u16, body: Value) -> Reply {
    (method, path, status, body, Duration::ZERO, None)
}
fn version() -> Reply {
    reply("GET", "/version".into(), 200, json!({"ApiVersion":"1.53"}))
}

#[tokio::test]
async fn foreign_volume_is_never_adopted_or_mutated() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut foreign = volume(&env);
    foreign["Labels"]["io.factory.user"] = json!(Uuid::new_v4().to_string());
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply("GET", format!("/volumes/{}", env.volume_name), 200, foreign),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.create_volume(&env).await,
        Err(EnvironmentError::OwnershipConflict)
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn foreign_container_cannot_be_stopped() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut foreign = container(&env);
    foreign["Config"]["Labels"]["io.factory.image"] = json!("sha256:foreign");
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                foreign,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.stop(&env).await,
        Err(EnvironmentError::OwnershipConflict)
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn operation_deadline_covers_negotiation_and_never_reports_success() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut slow = version();
    slow.4 = Duration::from_millis(200);
    let fixture = fixture(metadata, vec![slow], Duration::from_millis(20)).await;
    assert_eq!(
        fixture.runtime.start(&env).await,
        Err(EnvironmentError::Timeout)
    );
}

#[tokio::test]
async fn missing_recorded_volume_is_never_recreated() {
    let metadata = metadata();
    let mut env = environment(&metadata);
    env.provisioning_state = ProvisioningState::VolumeReady;
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                404,
                json!({"message":"missing"}),
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.inspect(&env).await,
        Err(EnvironmentError::VolumeMissing)
    );
}

#[tokio::test]
async fn old_policy_owned_container_can_still_be_stopped_by_immutable_id() {
    let metadata = metadata();
    let mut env = environment(&metadata);
    env.policy_version = "old-policy".into();
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                container(&env),
            ),
            reply(
                "POST",
                format!("/containers/{}/stop", "b".repeat(64)),
                204,
                Value::Null,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(fixture.runtime.stop(&env).await, Ok(()));
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[test]
fn full_policy_rejects_unsafe_mutations_and_finite_resource_ceilings() {
    let env = environment(&metadata());
    let base = container(&env);
    assert!(policy().verify_container(&env, &base).is_ok());
    let mut normalized = base.clone();
    normalized["HostConfig"]["PidsLimit"] = Value::Null;
    assert!(policy().verify_container(&env, &normalized).is_ok());
    for (path, value) in [
        ("/HostConfig/Privileged", json!(true)),
        ("/HostConfig/Memory", json!(1073741824)),
        ("/HostConfig/MemorySwap", json!(1073741824)),
        ("/HostConfig/NanoCpus", json!(1000000000)),
        ("/HostConfig/PidsLimit", json!(128)),
        ("/HostConfig/Annotations", json!({})),
        (
            "/HostConfig/Annotations/org.systemd.property.TasksMax",
            json!("uint64 128"),
        ),
        ("/HostConfig/ReadonlyRootfs", json!(false)),
        ("/HostConfig/NetworkMode", json!("host")),
        ("/HostConfig/CapDrop", json!([])),
        ("/HostConfig/Init", json!(false)),
        ("/HostConfig/SecurityOpt", json!(["seccomp=unconfined"])),
        ("/Config/User", json!("root")),
        ("/Config/Env", json!(["SECRET=bad"])),
        ("/Mounts/0/Name", json!("foreign")),
        ("/HostConfig/Mounts/0/Source", json!("foreign")),
    ] {
        let mut changed = base.clone();
        *changed.pointer_mut(path).unwrap() = value;
        assert_eq!(
            policy().verify_container(&env, &changed),
            Err(EnvironmentError::PolicyMismatch),
            "{path}"
        );
    }
}

#[tokio::test]
async fn stopped_intent_can_inspect_owned_container_even_when_volume_is_missing() {
    let metadata = metadata();
    let mut env = environment(&metadata);
    env.provisioning_state = ProvisioningState::VolumeReady;
    env.desired_state = DesiredState::Stopped;
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                404,
                json!({"message":"missing"}),
            ),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                container(&env),
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.inspect(&env).await,
        Ok(RuntimeInspection {
            state: ObservedState::Running,
            volume: VolumePresence::Missing
        })
    );
}

#[tokio::test]
async fn running_observation_requires_complete_policy() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut unsafe_container = container(&env);
    unsafe_container["HostConfig"]["Binds"] = json!(["/var/run/docker.sock:/var/run/docker.sock"]);
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                200,
                volume(&env),
            ),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                unsafe_container,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.inspect(&env).await,
        Err(EnvironmentError::PolicyMismatch)
    );
}

#[tokio::test]
async fn create_conflict_requires_reinspection_and_never_starts_foreign_container() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut foreign = container(&env);
    foreign["Config"]["Labels"]["io.factory.user"] = json!(Uuid::new_v4().to_string());
    let image = json!({"Id":env.image_id,"Os":"linux","Config":{"User":"10000:10000","Labels":{"io.factory.environment.image":POLICY_VERSION}}});
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                200,
                volume(&env),
            ),
            reply("GET", format!("/images/{}/json", env.image_id), 200, image),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                404,
                json!({"message":"missing"}),
            ),
            reply(
                "POST",
                "/containers/create".into(),
                409,
                json!({"message":"conflict"}),
            ),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                foreign,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.start(&env).await,
        Err(EnvironmentError::OwnershipConflict)
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[test]
fn preparation_rejects_tag_and_oversized_metadata() {
    let directory = std::env::temp_dir().join(format!("factory-env-metadata-{}", Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    for bytes in [json!({"schemaVersion":1,"installationId":Uuid::new_v4().to_string(),"imageId":"alpine:latest","policyVersion":"2"}).to_string(), " ".repeat(4097)] {
        std::fs::write(directory.join("prepared.json"),bytes).unwrap();
        assert!(matches!(PreparationMetadata::load(&directory),Err(EnvironmentError::InvalidConfiguration)));
    }
    std::fs::remove_file(directory.join("prepared.json")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[tokio::test]
async fn supervisor_loss_after_creation_prevents_the_later_start_request() {
    let metadata = metadata();
    let env = environment(&metadata);
    let permission = Arc::new(AtomicBool::new(true));
    let mut created = container(&env);
    created["State"] = json!({"Status":"created", "Running":false});
    let mut create_reply = reply(
        "POST",
        "/containers/create".into(),
        201,
        json!({"Id":"b".repeat(64), "Warnings":[]}),
    );
    create_reply.5 = Some(permission.clone());
    let image = json!({"Id":env.image_id,"Os":"linux","Config":{"User":"10000:10000","Labels":{"io.factory.environment.image":POLICY_VERSION}}});
    let mut fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                200,
                volume(&env),
            ),
            reply("GET", format!("/images/{}/json", env.image_id), 200, image),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                404,
                json!({"message":"missing"}),
            ),
            create_reply,
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                created,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    fixture.runtime.mutation_permitted = Arc::new(move || permission.load(Ordering::SeqCst));
    assert_eq!(
        fixture.runtime.start(&env).await,
        Err(EnvironmentError::Unavailable)
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn lost_supervisor_prevents_volume_creation_and_stop_mutations() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                404,
                json!({"message":"missing"}),
            ),
            version(),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                200,
                container(&env),
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    fixture.runtime.mutation_permitted = Arc::new(|| false);
    assert_eq!(
        fixture.runtime.create_volume(&env).await,
        Err(EnvironmentError::Unavailable)
    );
    assert_eq!(
        fixture.runtime.stop(&env).await,
        Err(EnvironmentError::Unavailable)
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn lost_supervisor_prevents_container_creation_after_read_only_checks() {
    let metadata = metadata();
    let env = environment(&metadata);
    let image = json!({"Id":env.image_id,"Os":"linux","Config":{"User":"10000:10000","Labels":{"io.factory.environment.image":POLICY_VERSION}}});
    let mut fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                200,
                volume(&env),
            ),
            reply("GET", format!("/images/{}/json", env.image_id), 200, image),
            reply(
                "GET",
                format!("/containers/{}/json", env.container_name),
                404,
                json!({"message":"missing"}),
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    fixture.runtime.mutation_permitted = Arc::new(|| false);
    assert_eq!(
        fixture.runtime.start(&env).await,
        Err(EnvironmentError::Unavailable)
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 4);
}

#[test]
fn installation_lock_remains_exclusive_until_guard_drops() {
    let directory = std::env::temp_dir().join(format!("factory-env-lock-{}", Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let guard = InstallationGuard::acquire(&directory).unwrap();
    assert!(InstallationGuard::acquire(&directory).is_err());
    drop(guard);
    drop(InstallationGuard::acquire(&directory).unwrap());
    std::fs::remove_file(directory.join("supervisor.lock")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[tokio::test]
#[ignore = "explicit local Docker fixture: FACTORY_ENV_TEST_DIRECTORY must contain a prepared image"]
async fn real_docker_persistence_isolation_and_unlimited_resources() {
    let directory =
        std::env::var("FACTORY_ENV_TEST_DIRECTORY").expect("explicit fixture directory required");
    let prepared = PreparationMetadata::load(&directory).unwrap();
    // Every test gets a fresh installation identity, even when reusing a prepared image.
    let metadata: PreparationMetadata=serde_json::from_value(json!({"schemaVersion":1,"installationId":Uuid::new_v4().to_string(),"imageId":prepared.image_id(),"policyVersion":"2"})).unwrap();
    let runtime = DockerEnvironmentRuntime::connect(metadata.clone(), Arc::new(|| true)).unwrap();
    let environments = [environment(&metadata), environment(&metadata)];
    let result = exercise_real(&runtime, &environments, &directory).await;
    // Exact generated resources only, with independent ownership verification before cleanup.
    for env in &environments {
        if let Some(container) = runtime
            .container(env)
            .await
            .expect("cleanup ownership inspection")
        {
            runtime
                .docker
                .remove_container(
                    container["Id"].as_str().unwrap(),
                    Some(bollard::query_parameters::RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
        }
        if runtime.volume(env).await.unwrap() == VolumePresence::Present {
            runtime
                .docker
                .remove_volume(
                    &env.volume_name,
                    None::<bollard::query_parameters::RemoveVolumeOptions>,
                )
                .await
                .unwrap();
        }
    }
    result.unwrap();
}

async fn exercise_real(
    runtime: &DockerEnvironmentRuntime,
    environments: &[Environment; 2],
    directory: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for env in environments {
        runtime.create_volume(env).await?;
        if let Err(error) = runtime.start(env).await {
            if let Some(container) = runtime.container(env).await? {
                for key in ["Memory", "MemorySwap", "NanoCpus", "PidsLimit"] {
                    eprintln!("owned fixture {key}: {}", container["HostConfig"][key]);
                }
            }
            return Err(error.into());
        }
        if runtime.inspect(env).await?.state != ObservedState::Running {
            return Err("not running".into());
        }
    }
    let first = &environments[0];
    let second = &environments[1];
    fixture_exec(
        directory,
        &first.container_name,
        "printf persistent > /home/factory/Documents/witness",
    )?;
    fixture_exec(
        directory,
        &second.container_name,
        "test ! -e /home/factory/Documents/witness",
    )?;
    runtime.stop(first).await?;
    runtime.start(first).await?;
    fixture_exec(
        directory,
        &first.container_name,
        "test \"$(cat /home/factory/Documents/witness)\" = persistent; test \"$(id -u)\" = 10000; test \"$(cat /sys/fs/cgroup/memory.max)\" = max; test \"$(cat /sys/fs/cgroup/pids.max)\" = max; test \"$(cat /sys/fs/cgroup/memory.swap.max)\" = max; test \"$(cat /sys/fs/cgroup/cpu.max)\" = 'max 100000'; test ! -w /etc/passwd; test ! -S /var/run/docker.sock",
    )?;
    let mut foreign = first.clone();
    foreign.user_id = Uuid::new_v4();
    if runtime.stop(&foreign).await != Err(EnvironmentError::OwnershipConflict) {
        return Err("foreign stop allowed".into());
    }
    for env in environments {
        runtime.stop(env).await?;
        runtime.stop(env).await?;
        if runtime.inspect(env).await?.state != ObservedState::Stopped {
            return Err("not stopped".into());
        }
    }
    Ok(())
}

fn fixture_exec(
    directory: &str,
    name: &str,
    command: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let host = if cfg!(windows) {
        "npipe:////./pipe/docker_engine"
    } else {
        "unix:///var/run/docker.sock"
    };
    let mut process = std::process::Command::new("docker");
    process.env_clear();
    for key in ["PATH", "SystemRoot", "ProgramFiles"] {
        if let Some(value) = std::env::var_os(key) {
            process.env(key, value);
        }
    }
    // The explicit fixture metadata directory contains no Docker credential config.
    let output = process
        .args([
            "--config", directory, "--host", host, "exec", name, "sh", "-exc", command,
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "fixture exec failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}
