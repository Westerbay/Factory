//! Real PostgreSQL recovery; the guarded runner supplies an isolated owned database.
use super::{EnvironmentHostFactory, HostRouter, recovery};
use crate::config::RuntimeConfig;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use factory_adapters::environments::{InstallationGuard, POLICY_VERSION};
use factory_application::environments::EnvironmentService;
use factory_core::{
    attachments::{AttachmentError, AttachmentStore},
    environments::{EnvironmentActor, EnvironmentError},
    identity::CredentialHash,
};
use factory_database::{identity::IdentityDatabase, local_target::from_fixture};
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};
use tower::ServiceExt;
use uuid::Uuid;

const DEADLINE: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
struct Fixture {
    project: String,
    owner: String,
    port: u16,
}

async fn database() -> (Arc<IdentityDatabase>, PgPool) {
    let path = std::env::var("FACTORY_TEST_TARGET_PATH").expect("explicit owned fixture target");
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert!(metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 4096);
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = from_fixture(&root, &fixture.project, &fixture.owner, fixture.port)
        .await
        .unwrap();
    let database = Arc::new(IdentityDatabase::connect(&target).await.unwrap());
    let options = PgConnectOptions::new_without_pgpass()
        .host(target.host())
        .port(target.port())
        .database(target.database())
        .username(target.username())
        .password(target.password())
        .ssl_mode(sqlx::postgres::PgSslMode::Disable);
    let sql = PgPool::connect_with(options).await.unwrap();
    (database, sql)
}

fn workspace(installation: Uuid) -> (PathBuf, RuntimeConfig) {
    let root = std::env::temp_dir().join(format!("factory-supervisor-recovery-{installation}"));
    std::fs::create_dir(&root).unwrap();
    println!("Supervisor recovery fixture workspace: {}", root.display());
    let directory = root.join(".factory/environments");
    std::fs::create_dir_all(&directory).unwrap();
    // No environment belongs to this fresh installation. The host only executes
    // empty database sweeps, so these immutable fixture IDs never reach Docker.
    for (name, value) in [
        (
            "prepared.json",
            json!({
                "schemaVersion": 1,
                "installationId": installation,
                "imageId": format!("sha256:{}", "a".repeat(64)),
                "policyVersion": POLICY_VERSION,
            }),
        ),
        (
            "egress.json",
            json!({
                "schemaVersion": 1,
                "installationId": installation,
                "imageId": format!("sha256:{}", "b".repeat(64)),
                "hostIpv4": ["127.0.0.1"],
            }),
        ),
    ] {
        std::fs::write(directory.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let config = RuntimeConfig::from_values(|key| {
        (key == "FACTORY_WORKSPACE_ROOT").then(|| root.to_str().unwrap().to_owned())
    })
    .unwrap();
    (root, config)
}

fn runtime_router(service: Arc<EnvironmentService>, epoch: &'static str) -> Router {
    // This marker endpoint exists only in this ignored integration-test binary.
    // It observes the production service without exposing a fixture route in the server.
    Router::new().route(
        "/runtime",
        get(move || {
            let service = service.clone();
            async move {
                let status = if service.is_available() {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                (status, epoch)
            }
        }),
    )
}

async fn response(router: &Router) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/runtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 128).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn expect_response(router: &Router, status: StatusCode, epoch: &str) {
    tokio::time::timeout(DEADLINE, async {
        loop {
            if response(router).await == (status, epoch.to_owned()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the live recovery router must reach the expected state");
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn lost_postgres_supervisor_recovers_without_restarting_the_router() {
    let (database, sql) = database().await;
    let installation = Uuid::new_v4();
    let (workspace, config) = workspace(installation);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM factory_identity.environments WHERE installation_id=$1",
    )
    .bind(installation)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(count, 0, "the fixture must not dispatch any Docker work");
    let factory = EnvironmentHostFactory::prepare(&config, database.clone())
        .await
        .unwrap();
    let initial = factory.start().await.unwrap();
    // The host composes attachments over the real schema: an unknown session is
    // rejected by the store's own queries, never reported as unavailable.
    let stranger = EnvironmentActor {
        user_id: Uuid::new_v4(),
        session_hash: CredentialHash([0; 32]),
    };
    let rejected = database
        .attachment_store()
        .get(stranger, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(
        matches!(
            rejected,
            AttachmentError::Unauthorized | AttachmentError::NotFound
        ),
        "{rejected:?}"
    );
    assert!(
        initial
            .attachments()
            .get(stranger, Uuid::new_v4())
            .await
            .is_err()
    );
    let old_service = initial.service();
    let old_router = runtime_router(old_service.clone(), "original");
    let (stop, stopped) = watch::channel(false);
    let (attempted, mut failures) = mpsc::unbounded_channel();
    let (created, mut replacements) = mpsc::unbounded_channel();
    // Hold only the first replacement attempt until the competing guard is held.
    // This makes the exclusivity race deterministic without changing production timeouts.
    let (allow_attempt, first_attempt) = oneshot::channel();
    let mut first_attempt = Some(first_attempt);
    let replacing = factory.clone();
    let (router, worker) = recovery::recovering_router(
        Router::new(),
        Some(HostRouter {
            router: old_router.clone(),
            host: initial,
        }),
        move || {
            let gate = first_attempt.take();
            let factory = replacing.clone();
            let attempted = attempted.clone();
            let created = created.clone();
            async move {
                if let Some(gate) = gate {
                    gate.await.unwrap();
                }
                match factory.start().await {
                    Ok(host) => {
                        let service = host.service();
                        created.send(service.clone()).unwrap();
                        Ok(HostRouter {
                            router: runtime_router(service, "replacement"),
                            host,
                        })
                    }
                    Err(error) => {
                        attempted.send(error).unwrap();
                        Err(error)
                    }
                }
            }
        },
        stopped,
    );
    expect_response(&router, StatusCode::OK, "original").await;

    // A bigint advisory key is stored as classid/objid with objsubid=1. Restrict
    // termination to this installation's granted exclusive lock in the verified DB.
    let terminated: Vec<bool> = sqlx::query_scalar(
        "SELECT pg_terminate_backend(pid) FROM pg_locks \
         WHERE locktype='advisory' AND mode='ExclusiveLock' AND granted \
         AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) \
         AND objsubid=1 AND ((classid::bigint << 32) | objid::bigint)=hashtextextended($1,61925) \
         AND pid<>pg_backend_pid()",
    )
    .bind(installation.to_string())
    .fetch_all(&sql)
    .await
    .unwrap();
    assert_eq!(terminated, vec![true]);
    let competitor = tokio::time::timeout(DEADLINE, async {
        loop {
            match database.environment_supervisor(installation).await {
                Ok(guard) => break guard,
                Err(EnvironmentError::Conflict) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("unexpected fixture guard error: {error}"),
            }
        }
    })
    .await
    .unwrap();
    expect_response(&router, StatusCode::SERVICE_UNAVAILABLE, "original").await;
    assert!(
        EnvironmentHostFactory::prepare(&config, database.clone())
            .await
            .is_err()
    );
    allow_attempt.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(DEADLINE, failures.recv())
            .await
            .unwrap(),
        Some(EnvironmentError::Conflict)
    );
    assert!(replacements.try_recv().is_err());
    assert!(!old_service.is_available());
    assert_eq!(response(&router).await.0, StatusCode::SERVICE_UNAVAILABLE);
    competitor.close().await.unwrap();

    let new_service = tokio::time::timeout(DEADLINE, replacements.recv())
        .await
        .unwrap()
        .unwrap();
    expect_response(&router, StatusCode::OK, "replacement").await;
    assert!(!Arc::ptr_eq(&old_service, &new_service));
    assert!(!old_service.is_available());
    assert_eq!(
        response(&old_router).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        EnvironmentHostFactory::prepare(&config, database.clone())
            .await
            .is_err()
    );

    factory.close();
    assert!(
        !new_service.is_available(),
        "shutdown must revoke admission immediately"
    );
    assert!(matches!(
        factory.start().await,
        Err(EnvironmentError::Unavailable)
    ));
    stop.send_replace(true);
    tokio::time::timeout(DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response(&router).await.0, StatusCode::SERVICE_UNAVAILABLE);
    let released = database.environment_supervisor(installation).await.unwrap();
    released.close().await.unwrap();
    assert!(InstallationGuard::acquire(workspace.join(".factory/environments")).is_err());
    drop(factory);
    drop(InstallationGuard::acquire(workspace.join(".factory/environments")).unwrap());
    database.close().await;
    sql.close().await;

    // The last process guard is gone before deleting this fixture's exact files.
    let directory = workspace.join(".factory/environments");
    for name in ["prepared.json", "egress.json", "supervisor.lock"] {
        std::fs::remove_file(directory.join(name)).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
    std::fs::remove_dir(workspace.join(".factory")).unwrap();
    std::fs::remove_dir(workspace).unwrap();
}
