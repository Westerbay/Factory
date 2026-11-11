use super::*;
use crate::egress::{DockerEgress, EgressMetadata, user_network_name};
use bollard::{API_DEFAULT_VERSION, Docker};
use chrono::Utc;
use factory_core::environments::*;
use futures_util::FutureExt;
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
    serde_json::from_value(json!({"schemaVersion":1,"installationId":Uuid::new_v4().to_string(),"imageId":format!("sha256:{}", "a".repeat(64)),"policyVersion":POLICY_VERSION})).unwrap()
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
        policy_version: POLICY_VERSION.into(),
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
        "Mounts":[{"Type":"volume","Name":env.volume_name,"Destination":"/home/factory","RW":true,"Driver":"local"}],
        "NetworkSettings":{"Networks":{user_network_name(env.installation_id,env.id):{"NetworkID":"e".repeat(64)}}}})
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
    egress: EgressMetadata,
}

fn egress_reply(state: &FixtureState, path: &str) -> Option<Value> {
    let metadata = &state.egress;
    if path.ends_with(&format!("/images/{}/json", metadata.image_id)) {
        return Some(
            json!({"Id":metadata.image_id,"Os":"linux","Config":{"Labels":{"io.factory.egress.image":"1","io.factory.egress.squid":"7.7"}}}),
        );
    }
    if path.ends_with(&format!("/containers/{}/json", metadata.proxy_name())) {
        let body = serde_json::to_value(metadata.proxy_body()).unwrap();
        return Some(
            json!({"Id":"c".repeat(64),"Name":format!("/{}",metadata.proxy_name()),"Image":metadata.image_id,
            "Config":body,"HostConfig":body["HostConfig"],"State":{"Running":true},"Mounts":[],
            "NetworkSettings":{"Networks":{metadata.outbound_name():{"NetworkID":"d".repeat(64)}}}}),
        );
    }
    if path.contains("/networks/") {
        let name = path.rsplit('/').next().unwrap();
        let environment = if name == metadata.outbound_name() {
            None
        } else {
            Some(
                Uuid::parse_str(
                    name.strip_prefix(&format!("factory-net-{}-", metadata.installation_id))
                        .unwrap(),
                )
                .unwrap(),
            )
        };
        let mut network = serde_json::to_value(metadata.network_body(environment)).unwrap();
        network["Id"] = json!(if environment.is_some() {
            "e".repeat(64)
        } else {
            "d".repeat(64)
        });
        network["Scope"] = json!("local");
        network["IPAM"] = json!({"Driver":"default","Options":{}});
        network["Containers"] = json!({"c".repeat(64):{"Name":metadata.proxy_name()}});
        return Some(network);
    }
    if path.ends_with("/version")
        && state
            .replies
            .lock()
            .unwrap()
            .front()
            .is_none_or(|reply| reply.1 != "/version")
    {
        return Some(json!({"ApiVersion":"1.53","Os":"linux"}));
    }
    None
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
    let egress_metadata = super::fixture::synthetic_egress(metadata.installation_id());
    let router = Router::new()
        .fallback(any(
            |State(state): State<FixtureState>, request: Request<Body>| async move {
                if let Some(body) = egress_reply(&state, request.uri().path()) {
                    assert_eq!(request.method(), "GET", "mock egress is already prepared");
                    return Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap();
                }
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
            egress: egress_metadata.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    // HTTP exists only in this private test constructor; production exposes local transport only.
    let docker =
        Docker::connect_with_http(&format!("http://{address}"), 5, API_DEFAULT_VERSION).unwrap();
    Fixture {
        runtime: DockerEnvironmentRuntime {
            egress: Arc::new(
                DockerEgress::new(docker.clone(), egress_metadata, Arc::new(|| true)).unwrap(),
            ),
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
async fn volume_with_invalid_image_label_or_mismatched_policy_is_never_adopted() {
    for label in ["io.factory.image", "io.factory.policy"] {
        let metadata = metadata();
        let env = environment(&metadata);
        let mut foreign = volume(&env);
        foreign["Labels"][label] = json!("different");
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
}

#[tokio::test]
async fn upgraded_workspace_reuses_the_owned_home_from_its_previous_image() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut old_volume = volume(&env);
    old_volume["Labels"]["io.factory.image"] = json!(format!("sha256:{}", "f".repeat(64)));
    let fixture = fixture(
        metadata,
        vec![
            version(),
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                200,
                old_volume,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    fixture.runtime.create_volume(&env).await.unwrap();
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
async fn container_network_id_must_match() {
    let metadata = metadata();
    let env = environment(&metadata);
    let mut wrong_network = container(&env);
    wrong_network["NetworkSettings"]["Networks"][user_network_name(env.installation_id, env.id)]
        ["NetworkID"] = json!("0".repeat(64));
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
                wrong_network,
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        fixture.runtime.inspect(&env).await,
        Err(EnvironmentError::OwnershipConflict)
    );
    assert!(
        fixture.replies.lock().unwrap().is_empty(),
        "mismatched network must be rejected"
    );
}

#[test]
fn egress_metadata_cannot_select_another_installation() {
    let metadata = metadata();
    let other = super::fixture::synthetic_egress(Uuid::new_v4());
    assert!(matches!(
        DockerEnvironmentRuntime::connect(metadata, other, Arc::new(|| true)),
        Err(EnvironmentError::OwnershipConflict)
    ));
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
            reply("GET", "/containers/json".into(), 200, json!([])),
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
    assert_eq!(
        base["Config"]["Entrypoint"],
        json!(["/usr/local/bin/factory-project-controller"])
    );
    assert_eq!(base["Config"]["Cmd"], json!(["--bootstrap"]));
    assert!(policy().verify_container(&env, &base).is_ok());
    let mut normalized = base.clone();
    normalized["HostConfig"]["PidsLimit"] = Value::Null;
    normalized["Config"]
        .as_object_mut()
        .unwrap()
        .remove("NetworkDisabled");
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
        ("/Config/Entrypoint", json!(["/bin/sleep"])),
        ("/Config/Cmd", json!(["infinity"])),
        ("/Config/NetworkDisabled", json!(true)),
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
    for bytes in [json!({"schemaVersion":1,"installationId":Uuid::new_v4().to_string(),"imageId":"alpine:latest","policyVersion":POLICY_VERSION}).to_string(), " ".repeat(4097)] {
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
    let metadata: PreparationMetadata=serde_json::from_value(json!({"schemaVersion":1,"installationId":Uuid::new_v4().to_string(),"imageId":prepared.image_id(),"policyVersion":POLICY_VERSION})).unwrap();
    let egress = super::fixture::egress_metadata(metadata.installation_id());
    let runtime =
        DockerEnvironmentRuntime::connect(metadata.clone(), egress, Arc::new(|| true)).unwrap();
    let environments = [environment(&metadata), environment(&metadata)];
    let result = std::panic::AssertUnwindSafe(exercise_real(&runtime, &environments, &directory))
        .catch_unwind()
        .await;
    super::fixture::cleanup(&runtime, &[&environments[0], &environments[1]]).await;
    result.unwrap().unwrap();
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

#[tokio::test]
async fn process_start_preflight_refusals_never_submit_an_exec() {
    use super::process::{FixedProcess, ProcessStartError};
    let metadata = metadata();
    let mut env = environment(&metadata);
    let fixture = fixture(metadata.clone(), vec![], Duration::from_millis(300)).await;
    env.desired_state = DesiredState::Stopped;
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::PreSubmission(EnvironmentError::Conflict))
    ));
    env.desired_state = DesiredState::Running;
    env.policy_version = "old".into();
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::PreSubmission(
            EnvironmentError::PolicyMismatch
        ))
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn process_start_missing_volume_and_failed_reads_are_pre_submission() {
    use super::process::{FixedProcess, ProcessStartError};
    for (status, expected) in [
        (404, EnvironmentError::VolumeMissing),
        (500, EnvironmentError::Unavailable),
    ] {
        let metadata = metadata();
        let env = environment(&metadata);
        let fixture = fixture(
            metadata,
            vec![reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                status,
                json!({"message":"fixture"}),
            )],
            Duration::from_secs(1),
        )
        .await;
        assert!(
            matches!(fixture.runtime.start_process_checked(&env, FixedProcess::CodexExecution).await,
            Err(ProcessStartError::PreSubmission(actual)) if actual == expected)
        );
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
        assert!(fixture.replies.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn process_start_stopped_container_is_pre_submission() {
    use super::process::{FixedProcess, ProcessStartError};
    let metadata = metadata();
    let env = environment(&metadata);
    let mut stopped = container(&env);
    stopped["State"] = json!({"Status":"exited","Running":false});
    let fixture = fixture(
        metadata,
        vec![
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
                stopped,
            ),
        ],
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::PreSubmission(
            EnvironmentError::Unavailable
        ))
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 2);
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_start_timeouts_and_transport_errors_keep_the_correct_phase() {
    use super::process::{FixedProcess, ProcessStartError};
    // Timeout while inspecting the volume cannot create or start an exec.
    let metadata = self::metadata();
    let env = environment(&metadata);
    let mut slow = reply(
        "GET",
        format!("/volumes/{}", env.volume_name),
        200,
        volume(&env),
    );
    slow.4 = Duration::from_millis(200);
    let fixture = self::fixture(metadata, vec![slow], Duration::from_millis(30)).await;
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::PreSubmission(EnvironmentError::Timeout))
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);

    // Docker has received create_exec: any transport error remains uncertain.
    let metadata = self::metadata();
    let env = environment(&metadata);
    let fixture = self::fixture(
        metadata,
        vec![
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
                container(&env),
            ),
            reply(
                "POST",
                format!("/containers/{}/exec", "b".repeat(64)),
                500,
                json!({"message":"fixture"}),
            ),
        ],
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::Submitted(EnvironmentError::Unavailable))
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 3);
    assert!(fixture.replies.lock().unwrap().is_empty());

    // Docker has received start_exec but delays its reply past the timeout.
    let metadata = self::metadata();
    let env = environment(&metadata);
    let mut slow_start = reply("POST", "/exec/fixture-exec/start".into(), 200, json!({}));
    slow_start.4 = Duration::from_millis(200);
    let fixture = self::fixture(
        metadata,
        vec![
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
                container(&env),
            ),
            reply(
                "POST",
                format!("/containers/{}/exec", "b".repeat(64)),
                200,
                json!({"Id":"fixture-exec"}),
            ),
            slow_start,
        ],
        Duration::from_millis(100),
    )
    .await;
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::Submitted(EnvironmentError::Timeout))
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 4);
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ownership_lost_during_process_preflight_never_creates_an_exec() {
    use super::process::{FixedProcess, ProcessStartError};
    let metadata = metadata();
    let env = environment(&metadata);
    let permission = Arc::new(AtomicBool::new(true));
    let mut inspection = reply(
        "GET",
        format!("/containers/{}/json", env.container_name),
        200,
        container(&env),
    );
    inspection.5 = Some(permission.clone());
    let mut fixture = fixture(
        metadata,
        vec![
            reply(
                "GET",
                format!("/volumes/{}", env.volume_name),
                200,
                volume(&env),
            ),
            inspection,
        ],
        Duration::from_secs(1),
    )
    .await;
    fixture.runtime.mutation_permitted = Arc::new(move || permission.load(Ordering::SeqCst));
    assert!(matches!(
        fixture
            .runtime
            .start_process_checked(&env, FixedProcess::CodexExecution)
            .await,
        Err(ProcessStartError::PreSubmission(
            EnvironmentError::Unavailable
        ))
    ));
    assert!(fixture.replies.lock().unwrap().is_empty());
    // The fixture rejects any unexpected POST, including a create_exec after inspection.
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 2);
}

fn cloned_runtime(fixture: &Fixture) -> Arc<DockerEnvironmentRuntime> {
    Arc::new(DockerEnvironmentRuntime {
        docker: fixture.runtime.docker.clone(),
        metadata: fixture.runtime.metadata.clone(),
        policy: policy(),
        timeout: fixture.runtime.timeout,
        mutation_permitted: fixture.runtime.mutation_permitted.clone(),
        egress: fixture.runtime.egress.clone(),
    })
}

#[tokio::test]
async fn codex_maps_process_phase_to_recoverable_or_uncertain_execution() {
    use crate::codex_execution::CodexExecutionAdapter;
    use factory_core::conversations::execution::{ExecutionError, ExecutionRuntime};

    let metadata = metadata();
    let env = environment(&metadata);
    let fixture = self::fixture(
        metadata,
        vec![reply(
            "GET",
            format!("/volumes/{}", env.volume_name),
            404,
            json!({"message":"missing"}),
        )],
        Duration::from_secs(2),
    )
    .await;
    let adapter = CodexExecutionAdapter::new(cloned_runtime(&fixture));
    assert!(matches!(
        adapter.models(&env).await,
        Err(ExecutionError::PreflightUnavailable)
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);

    let metadata = self::metadata();
    let env = environment(&metadata);
    let fixture = self::fixture(
        metadata,
        vec![
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
                container(&env),
            ),
            reply(
                "POST",
                format!("/containers/{}/exec", "b".repeat(64)),
                500,
                json!({"message":"fixture"}),
            ),
        ],
        Duration::from_secs(2),
    )
    .await;
    let adapter = CodexExecutionAdapter::new(cloned_runtime(&fixture));
    assert!(matches!(
        adapter
            .open(&env, None, "fixture-model", None, "hello")
            .await,
        Err(ExecutionError::LaunchUncertain)
    ));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 3);
}
