use factory_core::{environments::*, identity::*};
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

async fn fixture() -> (IdentityDatabase, PgPool) {
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
    sqlx::query("TRUNCATE factory_identity.users, factory_identity.login_attempts CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    (db, sql)
}

async fn actor(sql: &PgPool, token: u8, admin: bool) -> EnvironmentActor {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Synthetic fixture',$2,'active',clock_timestamp())")
        .bind(id).bind(if admin { "admin" } else { "member" }).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.external_identities(user_id,provider,issuer,subject,handle) VALUES($1,'test','https://fixture.invalid',$2,'fixture')")
        .bind(id).bind(id.to_string()).execute(sql).await.unwrap();
    let hash = CredentialHash([token; 32]);
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')")
        .bind(hash.0.as_slice()).bind(id).bind("c".repeat(32)).execute(sql).await.unwrap();
    EnvironmentActor {
        user_id: id,
        session_hash: hash,
    }
}

fn request(installation_id: Uuid) -> EnvironmentStart {
    EnvironmentStart {
        installation_id,
        image_id: "sha256:fixture".into(),
        policy_version: "1".into(),
    }
}

fn stopped() -> EnvironmentObservation {
    EnvironmentObservation {
        state: ObservedState::Stopped,
        error: None,
        volume_confirmed: true,
    }
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn persistent_requests_preserve_ownership_and_generation_without_capacity_or_presence() {
    let (db, sql) = fixture().await;
    let store = db.environment_store();
    let one = actor(&sql, 1, false).await;
    let two = actor(&sql, 2, false).await;
    let three = actor(&sql, 3, false).await;
    let installation = Uuid::new_v4();
    assert_eq!(store.get(one).await.unwrap(), None);
    assert_eq!(store.stop(one).await.unwrap(), None);
    let forged = EnvironmentActor {
        user_id: two.user_id,
        session_hash: one.session_hash,
    };
    assert!(matches!(
        store.start(forged, request(installation)).await,
        Err(EnvironmentError::Unauthorized)
    ));
    let (a, b) = tokio::join!(
        store.start(one, request(installation)),
        store.start(one, request(installation))
    );
    let first = a.unwrap();
    assert_eq!(first.id, b.unwrap().id);
    let (b, c) = tokio::join!(
        store.start(two, request(installation)),
        store.start(three, request(installation))
    );
    let second = b.unwrap();
    assert_ne!(first.volume_name, second.volume_name);
    assert_ne!(first.container_name, c.unwrap().container_name);
    assert!(matches!(
        store.start(one, request(Uuid::new_v4())).await,
        Err(EnvironmentError::OwnershipConflict)
    ));
    assert!(
        store
            .begin_provisioning(first.id, first.generation)
            .await
            .unwrap()
    );
    assert!(
        !store
            .begin_provisioning(first.id, first.generation)
            .await
            .unwrap()
    );
    assert_eq!(
        store.get(one).await.unwrap().unwrap().provisioning_state,
        ProvisioningState::VolumeCreating
    );
    let mut changed_image = request(installation);
    changed_image.image_id = "sha256:new".into();
    assert!(matches!(
        store.start(one, changed_image).await,
        Err(EnvironmentError::PolicyMismatch)
    ));
    let mut changed_policy = request(installation);
    changed_policy.policy_version = "next-policy".into();
    assert!(matches!(
        store.start(one, changed_policy).await,
        Err(EnvironmentError::PolicyMismatch)
    ));
    let unchanged = store.get(one).await.unwrap().unwrap();
    assert_eq!(unchanged.image_id, first.image_id);
    assert_eq!(unchanged.policy_version, first.policy_version);
    assert_eq!(unchanged.generation, first.generation);
    let stopping = store.stop(one).await.unwrap().unwrap();
    assert_eq!(stopping.generation, first.generation + 1);
    assert_eq!(
        store.stop(one).await.unwrap().unwrap().generation,
        stopping.generation
    );
    assert!(
        !store
            .record_observation(first.id, first.generation, stopped(), 0)
            .await
            .unwrap()
    );
    assert!(
        store
            .record_observation(first.id, stopping.generation, stopped(), 0)
            .await
            .unwrap()
    );
    assert_eq!(
        store.get(one).await.unwrap().unwrap().provisioning_state,
        ProvisioningState::VolumeReady
    );
    store.start(one, request(installation)).await.unwrap();
    // Simulate returning the next day: old browser metadata and expired login sessions
    // cannot stop a persistent environment or change its generation.
    sqlx::query("UPDATE factory_identity.environments SET next_reconcile_at=clock_timestamp()-interval '1 day' WHERE user_id=$1").bind(two.user_id).execute(&sql).await.unwrap();
    sqlx::query("UPDATE factory_identity.sessions SET expires_at=clock_timestamp()-interval '1 day' WHERE user_id=$1").bind(two.user_id).execute(&sql).await.unwrap();
    assert!(matches!(
        store.get(two).await,
        Err(EnvironmentError::Unauthorized)
    ));
    let persistent = store
        .reconcile_batch(installation, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.user_id == two.user_id)
        .unwrap();
    assert_eq!(persistent.desired_state, DesiredState::Running);
    assert_eq!(persistent.generation, second.generation);
    db.close().await;
    sql.close().await;
}

async fn wait_for_user_lock(sql: &PgPool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%FOR UPDATE OF u%'").fetch_one(sql).await.unwrap();
            if count > 0 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn initialize_serializes_first_creation_stop_and_session_revocation() {
    let (db, sql) = fixture().await;
    let store = db.environment_store();
    let user = actor(&sql, 7, false).await;
    let installation = Uuid::new_v4();
    let (a, b) = tokio::join!(
        store.initialize_if_absent(user, request(installation)),
        store.initialize_if_absent(user, request(installation)),
    );
    let first = a.unwrap();
    let second = b.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.generation, second.generation);
    let stopped = store.stop(user).await.unwrap().unwrap();
    let mut changed = request(installation);
    changed.policy_version = "new-policy".into();
    let after = store.initialize_if_absent(user, changed).await.unwrap();
    assert_eq!(after.id, stopped.id);
    assert_eq!(after.generation, stopped.generation);
    assert_eq!(after.desired_state, DesiredState::Stopped);
    assert_eq!(after.policy_version, stopped.policy_version);

    // Hold the user lock while committing a stop, so initialize must wait and
    // observe the stopped intent rather than revive the prior running one.
    let running = store.start(user, request(installation)).await.unwrap();
    let waiting_store = db.environment_store();
    let mut held = sql.begin().await.unwrap();
    sqlx::query("SELECT id FROM factory_identity.users WHERE id=$1 FOR UPDATE")
        .bind(user.user_id)
        .execute(&mut *held)
        .await
        .unwrap();
    sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped', generation=generation+1 WHERE user_id=$1")
        .bind(user.user_id).execute(&mut *held).await.unwrap();
    let task = tokio::spawn(async move {
        waiting_store
            .initialize_if_absent(user, request(installation))
            .await
    });
    wait_for_user_lock(&sql).await;
    held.commit().await.unwrap();
    let after_stop = task.await.unwrap().unwrap();
    assert_eq!(after_stop.desired_state, DesiredState::Stopped);
    assert_eq!(after_stop.generation, running.generation + 1);

    // A first initialization whose session is revoked while waiting must not insert.
    let revoked = actor(&sql, 8, false).await;
    let waiting_store = db.environment_store();
    let mut held = sql.begin().await.unwrap();
    sqlx::query("SELECT id FROM factory_identity.users WHERE id=$1 FOR UPDATE")
        .bind(revoked.user_id)
        .execute(&mut *held)
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        waiting_store
            .initialize_if_absent(revoked, request(installation))
            .await
    });
    wait_for_user_lock(&sql).await;
    sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
        .bind(revoked.user_id)
        .execute(&mut *held)
        .await
        .unwrap();
    held.commit().await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(EnvironmentError::Unauthorized)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM factory_identity.environments WHERE user_id=$1")
            .bind(revoked.user_id)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(count, 0);
    db.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn suspension_is_atomic_and_waiting_requests_recheck_live_session() {
    let (db, sql) = fixture().await;
    let store = db.environment_store();
    let member = actor(&sql, 4, false).await;
    let admin = actor(&sql, 5, true).await;
    let installation = Uuid::new_v4();
    let started = store.start(member, request(installation)).await.unwrap();
    // A failure after environment intent changes rolls back the entire suspension.
    sqlx::raw_sql("CREATE FUNCTION factory_identity.reject_environment_suspension() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic rollback'; END $$; CREATE TRIGGER reject_environment_suspension BEFORE INSERT ON factory_identity.access_events FOR EACH ROW EXECUTE FUNCTION factory_identity.reject_environment_suspension();")
        .execute(&sql).await.unwrap();
    let change = AccessRequest {
        target_id: member.user_id,
        status: AccessDecision::Suspended,
        expected_version: 1,
    };
    assert!(
        db.store()
            .change_access(admin.session_hash, change.clone())
            .await
            .is_err()
    );
    let after = store.get(member).await.unwrap().unwrap();
    assert_eq!(after.generation, started.generation);
    assert_eq!(after.desired_state, DesiredState::Running);
    sqlx::raw_sql("DROP TRIGGER reject_environment_suspension ON factory_identity.access_events; DROP FUNCTION factory_identity.reject_environment_suspension();").execute(&sql).await.unwrap();
    db.store()
        .change_access(admin.session_hash, change)
        .await
        .unwrap();
    assert!(matches!(
        store.start(member, request(installation)).await,
        Err(EnvironmentError::Unauthorized)
    ));
    assert!(matches!(
        store.get(member).await,
        Err(EnvironmentError::Unauthorized)
    ));
    let record = store
        .reconcile_batch(installation, 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(record.desired_state, DesiredState::Stopped);
    assert_eq!(record.generation, started.generation + 1);

    let waiting = actor(&sql, 6, false).await;
    let mut held = sql.begin().await.unwrap();
    sqlx::query("SELECT id FROM factory_identity.users WHERE id=$1 FOR UPDATE")
        .bind(waiting.user_id)
        .execute(&mut *held)
        .await
        .unwrap();
    let operation = tokio::spawn(async move { store.start(waiting, request(installation)).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%FOR UPDATE OF u%'").fetch_one(&sql).await.unwrap();
            if count > 0 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
        .bind(waiting.user_id)
        .execute(&mut *held)
        .await
        .unwrap();
    held.commit().await.unwrap();
    assert!(matches!(
        operation.await.unwrap(),
        Err(EnvironmentError::Unauthorized)
    ));
    db.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn supervisor_replacement_preserves_exclusivity_and_failed_guards_stay_failed() {
    let (db, sql) = fixture().await;
    let installation = Uuid::new_v4();
    let mut guard = db.environment_supervisor(installation).await.unwrap();
    assert!(matches!(
        db.environment_supervisor(installation).await,
        Err(EnvironmentError::Conflict)
    ));
    let pid: i32 = sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND query LIKE '%61925%' AND pid<>pg_backend_pid() AND state='idle'")
        .fetch_one(&sql).await.unwrap();
    guard.check().await.unwrap();
    // Only the dedicated supervisor backend in this verified disposable DB is terminated.
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&sql)
        .await
        .unwrap();
    assert!(killed);
    assert!(guard.check().await.is_err());
    assert!(
        guard.check().await.is_err(),
        "guard must never reconnect after losing its lock"
    );
    let mut competitor = db.environment_supervisor(installation).await.unwrap();
    assert!(guard.check().await.is_err());
    assert!(matches!(
        db.environment_supervisor(installation).await,
        Err(EnvironmentError::Conflict)
    ));
    competitor.check().await.unwrap();
    competitor.close().await.unwrap();

    let mut replacement = db.environment_supervisor(installation).await.unwrap();
    replacement.check().await.unwrap();
    assert!(
        guard.check().await.is_err(),
        "recovery must replace the guard, never reactivate the failed instance"
    );
    replacement.close().await.unwrap();
    db.close().await;
    sql.close().await;
}
