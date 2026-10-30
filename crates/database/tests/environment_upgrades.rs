use factory_core::{environments::*, identity::CredentialHash};
use factory_database::{identity::IdentityDatabase, local_target::from_fixture};
use serde::Deserialize;
use sqlx::{PgPool, postgres::PgConnectOptions};
use uuid::Uuid;

#[derive(Deserialize)]
struct Fixture {
    project: String,
    owner: String,
    port: u16,
}

async fn fixture() -> (IdentityDatabase, PgPool, EnvironmentActor, EnvironmentStart) {
    let path = std::env::var("FACTORY_TEST_TARGET_PATH").expect("explicit owned fixture target");
    assert!(std::fs::metadata(&path).unwrap().len() <= 4096);
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = from_fixture(&root, &fixture.project, &fixture.owner, fixture.port)
        .await
        .unwrap();
    let db = IdentityDatabase::connect(&target).await.unwrap();
    let options = PgConnectOptions::new_without_pgpass()
        .host(target.host())
        .port(target.port())
        .database(target.database())
        .username(target.username())
        .password(target.password())
        .ssl_mode(sqlx::postgres::PgSslMode::Disable);
    let sql = PgPool::connect_with(options).await.unwrap();
    db.migrate().await.unwrap();
    let user_id = Uuid::new_v4();
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let session_hash = CredentialHash(bytes);
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Upgrade fixture','member','active',clock_timestamp())")
        .bind(user_id).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')")
        .bind(session_hash.0.as_slice()).bind(user_id).bind("c".repeat(32)).execute(&sql).await.unwrap();
    let actor = EnvironmentActor {
        user_id,
        session_hash,
    };
    let config = EnvironmentStart {
        installation_id: Uuid::new_v4(),
        image_id: format!("sha256:{}", "a".repeat(64)),
        policy_version: "2".into(),
    };
    let store = db.environment_store();
    let env = store.start(actor, config.clone()).await.unwrap();
    assert!(
        store
            .begin_provisioning(env.id, env.generation)
            .await
            .unwrap()
    );
    assert!(
        store
            .record_observation(
                env.id,
                env.generation,
                EnvironmentObservation {
                    state: ObservedState::Stopped,
                    error: None,
                    volume_confirmed: true
                },
                0
            )
            .await
            .unwrap()
    );
    store.stop(actor).await.unwrap();
    (db, sql, actor, config)
}

fn target(config: &EnvironmentStart) -> EnvironmentStart {
    EnvironmentStart {
        installation_id: config.installation_id,
        image_id: format!("sha256:{}", "b".repeat(64)),
        policy_version: "3".into(),
    }
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn durable_upgrade_excludes_start_and_supervision_and_resumes_after_reconnect() {
    let (db, sql, actor, config) = fixture().await;
    let store = db.environment_store();
    let before = store
        .upgrade_snapshot(actor.user_id, config.installation_id)
        .await
        .unwrap();
    let mut guard = db
        .environment_supervisor(config.installation_id)
        .await
        .unwrap();
    guard.check().await.unwrap();
    assert!(matches!(
        db.environment_supervisor(config.installation_id).await,
        Err(EnvironmentError::Conflict)
    ));
    let old_id = "c".repeat(64);
    let intent = store
        .begin_upgrade(&before.environment, &old_id, &target(&config))
        .await
        .unwrap();
    assert_eq!(intent.generation, before.environment.generation + 1);
    assert!(matches!(
        store.start(actor, config.clone()).await,
        Err(EnvironmentError::Conflict)
    ));
    assert!(
        store
            .reconcile_batch(config.installation_id, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.stop(actor).await.unwrap().unwrap().generation,
        intent.generation
    );
    assert!(
        !store
            .record_observation(
                before.environment.id,
                before.environment.generation,
                EnvironmentObservation {
                    state: ObservedState::Running,
                    error: None,
                    volume_confirmed: false
                },
                0
            )
            .await
            .unwrap()
    );
    guard.close().await.unwrap();
    db.close().await;

    // The replacement process reads the same intent after losing all runtime objects.
    let path = std::env::var("FACTORY_TEST_TARGET_PATH").unwrap();
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let proof = from_fixture(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        &fixture.project,
        &fixture.owner,
        fixture.port,
    )
    .await
    .unwrap();
    let reopened = IdentityDatabase::connect(&proof).await.unwrap();
    let store = reopened.environment_store();
    let saved = store
        .upgrade_snapshot(actor.user_id, config.installation_id)
        .await
        .unwrap();
    assert_eq!(saved.pending.as_ref(), Some(&intent));
    assert_eq!(
        store
            .begin_upgrade(&saved.environment, &old_id, &target(&config))
            .await
            .unwrap(),
        intent
    );
    let mut wrong = intent.clone();
    wrong.generation += 1;
    assert_eq!(
        store.finish_upgrade(&wrong).await,
        Err(EnvironmentError::Conflict)
    );
    assert!(matches!(
        store
            .begin_upgrade(&saved.environment, &"d".repeat(64), &target(&config))
            .await,
        Err(EnvironmentError::Conflict)
    ));
    store.finish_upgrade(&intent).await.unwrap();
    let done = store
        .upgrade_snapshot(actor.user_id, config.installation_id)
        .await
        .unwrap();
    assert!(done.pending.is_none());
    assert_eq!(done.environment.image_id, target(&config).image_id);
    assert_eq!(done.environment.policy_version, "3");
    assert_eq!(done.environment.volume_name, before.environment.volume_name);
    assert_eq!(
        done.environment.container_name,
        before.environment.container_name
    );
    assert_eq!(
        done.environment.provisioning_state,
        ProvisioningState::VolumeReady
    );
    assert_eq!(done.environment.desired_state, DesiredState::Stopped);
    assert_eq!(done.environment.observed_state, ObservedState::NotCreated);
    assert_eq!(
        store
            .reconcile_batch(config.installation_id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    reopened.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn upgrade_compares_preview_under_the_same_user_lock_as_start() {
    let (db, sql, actor, config) = fixture().await;
    let store = db.environment_store();
    let stopped = store
        .upgrade_snapshot(actor.user_id, config.installation_id)
        .await
        .unwrap()
        .environment;
    let target = target(&config);
    let id = "c".repeat(64);
    let (start, upgrade) = tokio::join!(
        store.start(actor, config.clone()),
        store.begin_upgrade(&stopped, &id, &target)
    );
    assert_ne!(
        start.is_ok(),
        upgrade.is_ok(),
        "exactly one conflicting intent wins the lock"
    );
    let state = store
        .upgrade_snapshot(actor.user_id, config.installation_id)
        .await
        .unwrap();
    assert_eq!(state.pending.is_some(), upgrade.is_ok());
    assert_eq!(
        state.environment.desired_state == DesiredState::Running,
        start.is_ok()
    );
    assert!(matches!(
        store.upgrade_snapshot(actor.user_id, Uuid::new_v4()).await,
        Err(EnvironmentError::OwnershipConflict)
    ));
    db.close().await;
    sql.close().await;
}
