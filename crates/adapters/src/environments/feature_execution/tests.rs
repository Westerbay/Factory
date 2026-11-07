use super::*;
use crate::environments::{POLICY_VERSION, PreparationMetadata, fixture, policy::labels};
use bollard::exec::{CreateExecOptions, StartExecResults};
use chrono::Utc;
use factory_core::features::{AttemptClaim, AttemptStatus, FeatureTask, TaskRole, WorkspaceAccess};
use factory_core::{conversations::execution::ExecutionEvent, environments::*};
use futures_util::FutureExt;
use serde_json::json;

// Unsigned and nonfunctional. It only exercises the pinned local account parsers.
const FIXTURE_JWT: &str = "eyJhbGciOiJub25lIn0.eyJlbWFpbCI6InN5bnRoZXRpY0BleGFtcGxlLmludmFsaWQiLCJleHAiOjQxMDI0NDQ4MDAsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3BsYW5fdHlwZSI6InBybyIsImNoYXRncHRfdXNlcl9pZCI6ImZpeHR1cmUtdXNlciIsImNoYXRncHRfYWNjb3VudF9pZCI6ImZpeHR1cmUtYWNjb3VudCJ9fQ.fixture";

fn environment() -> (PreparationMetadata, Environment, Project) {
    let directory =
        std::env::var("FACTORY_ENV_TEST_DIRECTORY").expect("explicit prepared fixture directory");
    let prepared = PreparationMetadata::load(directory).unwrap();
    let installation = Uuid::new_v4();
    let metadata = serde_json::from_value(json!({
        "schemaVersion":1, "installationId":installation.to_string(),
        "imageId":prepared.image_id(), "policyVersion":POLICY_VERSION
    }))
    .unwrap();
    let id = Uuid::new_v4();
    let name = format!("factory-{}-{}", installation.simple(), id.simple());
    let user_id = Uuid::new_v4();
    let env = Environment {
        id,
        user_id,
        installation_id: installation,
        desired_state: DesiredState::Running,
        observed_state: ObservedState::NotCreated,
        generation: 1,
        provisioning_state: ProvisioningState::VolumeCreating,
        volume_name: format!("{name}-home"),
        container_name: name,
        image_id: prepared.image_id().to_owned(),
        policy_version: POLICY_VERSION.into(),
        observed_at: None,
        error_code: None,
        next_reconcile_at: Utc::now(),
    };
    let project = Project {
        id: Uuid::new_v4(),
        user_id,
        repository_id: 42,
        full_name: "fixture/project".into(),
        cloned_full_name: "fixture/project".into(),
        directory: "project".into(),
        state: ProjectState::Ready,
        operation_id: Uuid::new_v4(),
        environment_id: id,
        environment_generation: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    (metadata, env, project)
}

async fn execute(runtime: &DockerEnvironmentRuntime, id: &str, args: &[&str]) -> (i64, String) {
    let started = runtime
        .docker
        .create_exec(
            id,
            CreateExecOptions {
                cmd: Some(args.to_vec()),
                user: Some("10000:10000"),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let StartExecResults::Attached { mut output, .. } =
        runtime.docker.start_exec(&started.id, None).await.unwrap()
    else {
        panic!("fixture command must attach");
    };
    let mut log = String::new();
    while let Some(frame) = output.next().await {
        log.push_str(&frame.unwrap().to_string());
        assert!(log.len() < 32768, "fixture output must remain bounded");
    }
    (
        runtime
            .wait_process_exit(&started.id, Duration::from_secs(3))
            .await
            .unwrap(),
        log,
    )
}

#[tokio::test]
#[ignore = "explicit local Docker fixture: FACTORY_ENV_TEST_DIRECTORY with prepared pinned images"]
async fn isolated_workers_initialize_without_github_or_direct_network() {
    let (metadata, env, project) = environment();
    let egress = fixture::egress_metadata(metadata.installation_id());
    let runtime =
        Arc::new(DockerEnvironmentRuntime::connect(metadata, egress, Arc::new(|| true)).unwrap());
    let outcome = std::panic::AssertUnwindSafe(async {
        runtime.create_volume(&env).await.unwrap();
        // The relay test needs no external egress. Use a fixture-only main
        // container with the prepared image, an owned volume and no network.
        let main_fixture = runtime.docker.create_container(
            Some(CreateContainerOptions { name: Some(env.container_name.clone()), ..Default::default() }),
            serde_json::from_value(json!({
                "Image":env.image_id, "User":"10000:10000", "Entrypoint":["/bin/sleep","infinity"],
                "Labels":labels(&env),
                "HostConfig":{"NetworkMode":"none","ReadonlyRootfs":true,
                    "CapDrop":["ALL"], "SecurityOpt":["no-new-privileges"],
                    "Mounts":[{"Type":"volume","Source":env.volume_name,"Target":"/home/factory"}]}
            })).unwrap(),
        ).await.unwrap();
        runtime.docker.start_container(&main_fixture.id, None::<StartContainerOptions>).await.unwrap();
        let main = runtime.container(&env).await.unwrap().unwrap();
        let main_id = inspect_id(&main).unwrap();
        let feature_id = Uuid::new_v4();
        for (provider, model, access) in [
            ("codex", "gpt-5.4", WorkspaceAccess::ReadOnly),
            ("claude", "sonnet", WorkspaceAccess::Write),
        ] {
            let workspace = TaskWorkspace { feature_id, attempt_id: Uuid::new_v4(), base_oid: "a".repeat(40), access };
            let identity = WorkerIdentity::new(&env, &project, &workspace, env.generation, Role::Agent).unwrap();
            let setup = format!(r#"umask 077
mkdir -p /home/factory/.codex /home/factory/.claude /home/factory/.ssh /home/factory/{}
printf '%s' '{{"auth_mode":"chatgpt","last_refresh":"2099-11-07T00:00:00Z","tokens":{{"id_token":"{fixture_jwt}","access_token":"{fixture_jwt}","refresh_token":"must-not-copy","account_id":"fixture-account"}}}}' >/home/factory/.codex/auth.json
printf '%s' '{{"claudeAiOauth":{{"accessToken":"synthetic-fixture-not-a-real-key","refreshToken":"must-not-copy","expiresAt":4102444800000,"scopes":["user:inference","user:profile"],"subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}}}' >/home/factory/.claude/.credentials.json
printf witness >/home/factory/.ssh/witness
printf tracked >/home/factory/{}/tracked
python3 -c 'import json, os, sys; path=sys.argv[1]; info=os.stat(path); json.dump(dict(device=info.st_dev, inode=info.st_ino), open(os.path.join(os.path.dirname(path), "ready"), "w"))' /home/factory/{}
"#, identity.checkout, identity.checkout, identity.checkout, fixture_jwt = FIXTURE_JWT);
            assert_eq!(execute(&runtime, main_id, &["/bin/sh", "-ec", &setup]).await.0, 0);
            let proof_args = proof_command(&identity.checkout);
            let (proof_code, proof_output) = execute(&runtime, main_id, &proof_args.iter().map(String::as_str).collect::<Vec<_>>()).await;
            assert_eq!(proof_code, 0);
            let proof = parse_proof(&proof_output).unwrap();
            let request = FeatureRun { workspace: workspace.clone(), provider: provider.into(), model: model.into(), effort: None, input: "Fixture never begins a provider turn".into() };
            let provider = ScopedProvider::from_request(&request).unwrap();
            let relay = runtime.docker.create_exec(main_id, CreateExecOptions {
                cmd: Some(relay_command(provider, workspace.attempt_id)), user: Some("10000:10000".to_owned()),
                attach_stdin:Some(true), attach_stdout:Some(true), attach_stderr:Some(true), ..Default::default()
            }).await.unwrap();
            let StartExecResults::Attached { input, mut output } = runtime.docker.start_exec(&relay.id, None).await.unwrap() else {
                panic!("relay fixture must attach");
            };
            let Some(Ok(LogOutput::StdOut { message })) =
                tokio::time::timeout(Duration::from_secs(3), output.next()).await.unwrap()
            else {
                panic!("relay fixture did not provide mount identity");
            };
            let relay_identity = relay_proof(&message).unwrap().to_owned();
            let body = identity.body(&env, match provider { ScopedProvider::Codex => "/workspace", ScopedProvider::Claude => CHECKOUT }, provider.command(&request)).unwrap();
            let (process, exit) = runtime.create_feature_worker(&env, &identity, body, Some((DockerProcess { id: relay.id, input, output }, relay_identity)), proof).await.unwrap();
            let process_receipt = process.id.clone();
            let mut session = match provider {
                ScopedProvider::Codex => crate::codex_execution::scoped_session(process, exit, &request),
                ScopedProvider::Claude => crate::claude_execution::scoped_session(process, exit, &request),
            };
            let event = tokio::time::timeout(Duration::from_secs(35), session.output.next()).await;
            assert!(matches!(event, Ok(Ok(Some(ExecutionEvent::ThreadReady { .. })))),
                "{} native session initialization: {event:?}; worker state: {:?}",
                provider.as_str(), runtime.inspect_feature_worker(&process_receipt).await.unwrap().map(|value| value["State"].clone())
            );
            let probe = format!(r#"
import os, socket
assert os.path.isfile('/workspace/checkout/tracked')
assert not os.path.exists('/home/factory/.ssh/witness')
assert not os.path.exists('/home/factory/Documents')
try:
    socket.create_connection(('1.1.1.1',443),timeout=1)
    raise AssertionError('worker has direct network')
except OSError:
    pass
with socket.socket(socket.AF_UNIX) as channel:
    channel.settimeout(2)
    channel.connect('/run/factory-relay/relay.sock')
    channel.sendall(b'CONNECT ssh.github.com:443 HTTP/1.1\r\nHost: ssh.github.com:443\r\n\r\n')
    assert channel.recv(128).startswith(b'HTTP/1.1 403')
try:
    with open('/workspace/checkout/probe','w') as output:
        output.write('worker')
    assert {}
except OSError as error:
    assert error.errno == 30
    assert not {}
"#, i32::from(access == WorkspaceAccess::Write), i32::from(access == WorkspaceAccess::Write));
            let (code, log) = execute(&runtime, &process_receipt, &["/usr/bin/python3", "-c", &probe]).await;
            assert_eq!(code, 0, "{provider_name} fixture confinement: {log}", provider_name = provider.as_str());
            match provider {
                ScopedProvider::Codex => {
                    let (code, output) = execute(&runtime, &process_receipt, &[
                        "/opt/codex/bin/codex", "-c", "cli_auth_credentials_store=\"file\"", "login", "status"
                    ]).await;
                    assert_eq!(code, 0);
                    assert!(output.contains("ChatGPT"));
                }
                ScopedProvider::Claude => {
                    let (code, output) = execute(&runtime, &process_receipt, &[
                        "/opt/claude/claude", "auth", "status", "--json"
                    ]).await;
                    assert_eq!(code, 0);
                    let data: Value = serde_json::from_str(&output).unwrap();
                    assert_eq!(data["loggedIn"], true);
                    assert_eq!(data["authMethod"], "claude.ai");
                }
            }
            if matches!(provider, ScopedProvider::Codex) {
                drop(session.control);
                while let Some(event) = session.output.next().await.unwrap() {
                    if matches!(event, ExecutionEvent::Failed | ExecutionEvent::Uncertain) {
                        break;
                    }
                }
                // The application never acknowledged a saved launch. An exit
                // must remain available for recovery, including repeated reads.
                let feature_runtime = DockerFeatureExecutionRuntime::new(runtime.clone());
                let mut prior = TaskAttempt {
                    claim: AttemptClaim {
                        task: FeatureTask {
                            id: Uuid::new_v4(), title: "fixture".into(), instructions: "fixture".into(),
                            role: TaskRole::GeneralReview, paths: vec![], depends_on: vec![], checks: vec![],
                        },
                        revision: None, workspace: workspace.clone(), snapshot_id: None,
                        model: model.into(), effort: None, skill_revision: "fixture".into(), retry_of: None,
                    },
                    generation: env.generation, status: AttemptStatus::Uncertain, launch_settled: false,
                    process_receipt: None, provider_thread: None, observed_model: None, activity: None,
                    result: None, checkpoint: None, pending_verification: None,
                };
                let proof_observation = TaskObservation::StoppedWithWorkerProof { worker_id: process_receipt.clone() };
                for _ in 0..2 {
                    assert_eq!(feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(), proof_observation);
                }

                let check = WorkerIdentity::new(&env, &project, &workspace, env.generation, Role::Check).unwrap();
                let body = check_body(&check, &env, "exit 0").unwrap();
                let created = runtime.docker.create_container(
                    Some(CreateContainerOptions { name: Some(check.name.clone()), ..Default::default() }), body
                ).await.unwrap();
                assert_eq!(feature_runtime.reconcile_check(&env, &project, &workspace, env.generation).await.unwrap(), CheckObservation::Unknown);
                assert_eq!(feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(), TaskObservation::Unknown);
                assert!(runtime.inspect_feature_worker(&created.id).await.unwrap().is_some());
                runtime.stop_feature_workers(&env).await.unwrap();
                assert!(runtime.inspect_feature_worker(&created.id).await.unwrap().is_none());
                assert_eq!(feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(), proof_observation);
                prior.pending_verification = Some(workspace.attempt_id);
                prior.launch_settled = true;
                assert_eq!(feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(), TaskObservation::Unknown);
                prior.pending_verification = None;

                prior.process_receipt = Some(process_receipt.clone());
                assert_eq!(feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(), TaskObservation::Stopped);
                assert!(runtime.inspect_feature_worker(&process_receipt).await.unwrap().is_none());
                prior.launch_settled = false;
                prior.process_receipt = None;
                assert_eq!(feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(), TaskObservation::Unknown);
                let body = check_body(&check, &env, "printf should-not-run; exit 7").unwrap();
                let (process, exit) = runtime.create_feature_worker(&env, &check, body, None, "0:0").await.unwrap();
                let rejected = collect_check(process, exit).await.unwrap();
                assert_eq!(rejected.exit_code, 1);
                assert!(rejected.output.is_empty());
                feature_runtime.acknowledge_check(&env, &project, &workspace, env.generation).await.unwrap();
                let body = check_body(&check, &env, "printf observed; exit 7").unwrap();
                let (process, exit) = runtime.create_feature_worker(&env, &check, body, None, proof).await.unwrap();
                let check_id = process.id.clone();
                let output = collect_check(process, exit).await.unwrap();
                assert_eq!(output.exit_code, 7);
                assert_eq!(output.output, "observed");
                prior.pending_verification = Some(workspace.attempt_id);
                assert_eq!(
                    feature_runtime.reconcile_task(&env, &project, &prior).await.unwrap(),
                    TaskObservation::StoppedWithWorkerProof { worker_id: check_id.clone() }
                );
                let mut restarted = env.clone();
                restarted.generation += 1;
                assert!(feature_runtime.reconcile_check(&restarted, &project, &workspace, restarted.generation).await.is_err());
                assert_eq!(
                    feature_runtime.reconcile_check(&restarted, &project, &workspace, env.generation).await.unwrap(),
                    CheckObservation::Exited { exit_code: 7 }
                );
                assert!(runtime.inspect_feature_worker(&check_id).await.unwrap().is_some());
                for _ in 0..2 {
                    feature_runtime.acknowledge_check(&restarted, &project, &workspace, env.generation).await.unwrap();
                }
                assert!(runtime.inspect_feature_worker(&check_id).await.unwrap().is_none());
                assert_eq!(feature_runtime.reconcile_check(&restarted, &project, &workspace, env.generation).await.unwrap(), CheckObservation::Unknown);
            } else {
                runtime.docker.stop_container(main_id, Some(StopContainerOptions {
                    t: Some(2), ..Default::default()
                })).await.unwrap();
                let mut stopping = env.clone();
                stopping.desired_state = DesiredState::Stopped;
                assert_eq!(runtime.inspect(&stopping).await.unwrap().state, ObservedState::Stopping);
                runtime.stop(&stopping).await.unwrap();
                let stopped = runtime.inspect_feature_worker(&process_receipt).await.unwrap().unwrap();
                assert_eq!(stopped["State"]["Status"], "exited");
                assert_eq!(stopped["State"]["Running"], false);
                assert_eq!(runtime.inspect(&stopping).await.unwrap().state, ObservedState::Stopped);
            }
        }
    }).catch_unwind().await;
    runtime.stop_feature_workers(&env).await.unwrap();
    for id in runtime.list_feature_worker_ids(&env).await.unwrap() {
        if let Some(value) = runtime.inspect_feature_worker(&id).await.unwrap() {
            verify_worker_owner(&env, &value, &id).unwrap();
            runtime.remove_feature_worker(&id).await.unwrap();
        }
    }
    fixture::cleanup(&runtime, &[&env]).await;
    outcome.unwrap();
}
