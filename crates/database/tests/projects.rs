use factory_core::{
    environments::EnvironmentActor,
    identity::{CredentialHash, IdentityStore},
    projects::*,
};
use factory_database::{
    identity::IdentityDatabase, local_target::from_fixture, projects::PgProjectStore,
    schema::Readiness,
};
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
    let sql = PgPool::connect_with(
        PgConnectOptions::new_without_pgpass()
            .host(target.host())
            .port(target.port())
            .database(target.database())
            .username(target.username())
            .password(target.password())
            .ssl_mode(sqlx::postgres::PgSslMode::Disable),
    )
    .await
    .unwrap();
    (db, sql)
}
async fn actor(sql: &PgPool) -> (EnvironmentActor, Uuid) {
    let user = Uuid::new_v4();
    let env = Uuid::new_v4();
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Project test','member','active',clock_timestamp())").bind(user).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')").bind(bytes.as_slice()).bind(user).bind("c".repeat(32)).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,volume_name,container_name,image_id,policy_version) VALUES($1,$2,$3,'running','running',$4,$5,'image','1')").bind(env).bind(user).bind(Uuid::new_v4()).bind(format!("v-{env}")).bind(format!("c-{env}")).execute(sql).await.unwrap();
    (
        EnvironmentActor {
            user_id: user,
            session_hash: CredentialHash(bytes),
        },
        env,
    )
}
fn input(repository_id: i64, full_name: &str) -> ImportProject {
    ImportProject {
        repository_id,
        full_name: full_name.into(),
        operation_id: Uuid::new_v4(),
    }
}
#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn project_imports_are_owned_and_concurrent_reservations_are_idempotent() {
    let (db, sql) = fixture().await;
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    let store = PgProjectStore::new(sql.clone());
    let (one, env) = actor(&sql).await;
    let (two, env2) = actor(&sql).await;
    let a = input(100, "org/repo");
    let b = input(100, "org/repo");
    let (a, b) = tokio::join!(store.reserve(one, env, 1, a), store.reserve(one, env, 1, b));
    let (a, af) = a.unwrap();
    let (b, bf) = b.unwrap();
    assert_ne!(af, bf);
    assert_eq!(a.id, b.id);
    assert_eq!(a.directory, "repo");
    assert!(matches!(
        store.reserve(one, env, 1, input(101, "another/repo")).await,
        Err(ProjectError::DestinationExists)
    ));
    assert!(matches!(
        store.get(two, env2, 1, a.id).await,
        Err(ProjectError::NotFound)
    ));
    assert!(matches!(
        store.get(one, env, 2, a.id).await,
        Err(ProjectError::Conflict)
    ));
    let (renamed, fresh) = store
        .reserve(one, env, 1, input(100, "new-owner/new-name"))
        .await
        .unwrap();
    assert!(!fresh);
    assert_eq!(renamed.id, a.id);
    assert_eq!(renamed.directory, a.directory);
    assert_eq!(renamed.full_name, "new-owner/new-name");
    assert_eq!(renamed.cloned_full_name, "org/repo");
    assert!(matches!(
        store
            .retry(one, env, 1, a.id, a.operation_id, Uuid::new_v4())
            .await,
        Err(ProjectError::Conflict)
    ));
    assert!(matches!(
        store.settle(one, env, 1, a.id, Uuid::new_v4(), true).await,
        Err(ProjectError::Conflict)
    ));
    let failed = store
        .settle(one, env, 1, a.id, a.operation_id, false)
        .await
        .unwrap();
    assert_eq!(failed.state, ProjectState::Failed);
    let operation = Uuid::new_v4();
    let retry = store
        .retry(one, env, 1, a.id, a.operation_id, operation)
        .await
        .unwrap();
    assert_eq!(retry.state, ProjectState::Importing);
    assert_eq!(retry.cloned_full_name, "new-owner/new-name");
    assert!(matches!(
        store.settle(one, env, 1, a.id, a.operation_id, true).await,
        Err(ProjectError::Conflict)
    ));
    let ready = store
        .settle(one, env, 1, a.id, operation, true)
        .await
        .unwrap();
    assert_eq!(ready.state, ProjectState::Ready);
    assert_eq!(store.list(one, env, 1, None).await.unwrap().items.len(), 1);
    assert!(
        store
            .list(two, env2, 1, None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    // A generation change alone is insufficient; a later stopped observation is required.
    let (uncertain, fresh) = store
        .reserve(one, env, 1, input(101, "org/uncertain"))
        .await
        .unwrap();
    assert!(fresh);
    let (published, fresh) = store
        .reserve(one, env, 1, input(102, "org/published"))
        .await
        .unwrap();
    assert!(fresh);
    // The provider may rename or transfer a repository after the clone starts.
    let (renamed_published, fresh) = store
        .reserve(one, env, 1, input(102, "transferred/published"))
        .await
        .unwrap();
    assert!(!fresh);
    assert_eq!(renamed_published.full_name, "transferred/published");
    assert_eq!(renamed_published.cloned_full_name, "org/published");
    // The database never promotes an import that is still in flight.
    assert!(matches!(
        store
            .confirm_recovered(one, env, 1, published.id, published.operation_id)
            .await,
        Err(ProjectError::Conflict)
    ));
    let observed: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("UPDATE factory_identity.environments SET generation=2,desired_state='stopped',observed_state='stopped',observed_generation=2,observed_at=clock_timestamp() WHERE id=$1 RETURNING observed_at").bind(env).fetch_one(&sql).await.unwrap();
    assert!(matches!(
        store
            .recover_stopped(one, env, 1, observed, uncertain.id)
            .await,
        Err(ProjectError::Conflict)
    ));
    assert!(matches!(
        store
            .recover_stopped(two, env, 2, observed, uncertain.id)
            .await,
        Err(ProjectError::Conflict)
    ));
    assert!(matches!(
        store
            .recover_stopped(
                one,
                env,
                2,
                observed - chrono::Duration::seconds(1),
                uncertain.id
            )
            .await,
        Err(ProjectError::Conflict)
    ));
    let recovered = store
        .recover_stopped(one, env, 2, observed, uncertain.id)
        .await
        .unwrap();
    assert_eq!(recovered.state, ProjectState::Failed);
    let recovered_published = store
        .recover_stopped(one, env, 2, observed, published.id)
        .await
        .unwrap();
    assert_eq!(recovered_published.state, ProjectState::Failed);
    sqlx::query("UPDATE factory_identity.environments SET desired_state='running',observed_state='running' WHERE id=$1").bind(env).execute(&sql).await.unwrap();
    // This is internal-only: the caller must first verify the original marker.
    assert!(matches!(
        store
            .confirm_recovered(one, env, 2, published.id, Uuid::new_v4())
            .await,
        Err(ProjectError::Conflict)
    ));
    assert!(matches!(
        store
            .confirm_recovered(two, env2, 1, published.id, published.operation_id)
            .await,
        Err(ProjectError::Conflict)
    ));
    assert!(matches!(
        store
            .confirm_recovered(one, env, 1, published.id, published.operation_id)
            .await,
        Err(ProjectError::Conflict)
    ));
    let ready_published = store
        .confirm_recovered(one, env, 2, published.id, published.operation_id)
        .await
        .unwrap();
    assert_eq!(ready_published.state, ProjectState::Ready);
    assert_eq!(ready_published.operation_id, published.operation_id);
    assert_eq!(ready_published.directory, published.directory);
    assert_eq!(ready_published.full_name, "transferred/published");
    assert_eq!(ready_published.cloned_full_name, "org/published");
    assert!(matches!(
        store
            .confirm_recovered(one, env, 2, published.id, published.operation_id)
            .await,
        Err(ProjectError::Conflict)
    ));
    let new_operation = Uuid::new_v4();
    let retry = store
        .retry(
            one,
            env,
            2,
            uncertain.id,
            uncertain.operation_id,
            new_operation,
        )
        .await
        .unwrap();
    assert_eq!(retry.state, ProjectState::Importing);
    assert!(matches!(
        store
            .settle(one, env, 2, uncertain.id, uncertain.operation_id, true)
            .await,
        Err(ProjectError::Conflict)
    ));
    let bad_directory =
        sqlx::query("UPDATE factory_identity.projects SET directory='../unsafe' WHERE id=$1")
            .bind(a.id)
            .execute(&sql)
            .await;
    assert!(bad_directory.is_err());
    let cross_owner = sqlx::query(
        "INSERT INTO factory_identity.conversations(id,user_id,project_id) VALUES($1,$2,$3)",
    )
    .bind(Uuid::new_v4())
    .bind(two.user_id)
    .bind(a.id)
    .execute(&sql)
    .await;
    assert!(cross_owner.is_err());
    sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
        .bind(one.session_hash.0.as_slice())
        .execute(&sql)
        .await
        .unwrap();
    assert!(matches!(
        store.get(one, env, 2, a.id).await,
        Err(ProjectError::Unauthorized)
    ));
}
#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn readiness_rejects_old_schema_version() {
    let (db, sql) = fixture().await;
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    sqlx::query(
        "ALTER TABLE factory_identity.schema_info DROP CONSTRAINT schema_info_version_check",
    )
    .execute(&sql)
    .await
    .unwrap();
    sqlx::query("UPDATE factory_identity.schema_info SET version=1")
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Invalid);
    sqlx::query("UPDATE factory_identity.schema_info SET version=4")
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE factory_identity.schema_info ADD CONSTRAINT schema_info_version_check CHECK (version=4)").execute(&sql).await.unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn recoverable_import_batches_are_owned_available_when_stopped_and_reloadable() {
    let (_db, sql) = fixture().await;
    let store = PgProjectStore::new(sql.clone());
    let (one, env1) = actor(&sql).await;
    let (two, env2) = actor(&sql).await;
    let (one_pending, _) = store
        .reserve(one, env1, 1, input(700, "one/pending"))
        .await
        .unwrap();
    let (one_failed, _) = store
        .reserve(one, env1, 1, input(701, "one/failed"))
        .await
        .unwrap();
    store
        .settle(one, env1, 1, one_failed.id, one_failed.operation_id, false)
        .await
        .unwrap();
    let (two_pending, _) = store
        .reserve(two, env2, 1, input(702, "two/pending"))
        .await
        .unwrap();
    let observed: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("UPDATE factory_identity.environments SET generation=2,desired_state='stopped',observed_state='stopped',observed_generation=2,observed_at=clock_timestamp() WHERE id=$1 RETURNING observed_at").bind(env1).fetch_one(&sql).await.unwrap();
    assert_eq!(
        store.recoverable(one).await.unwrap(),
        vec![RecoverableImport {
            project_id: one_pending.id,
            operation_id: one_pending.operation_id,
            environment_id: env1,
            environment_generation: 1,
        }]
    );
    assert_eq!(
        store.recoverable(two).await.unwrap(),
        vec![RecoverableImport {
            project_id: two_pending.id,
            operation_id: two_pending.operation_id,
            environment_id: env2,
            environment_generation: 1,
        }]
    );
    let bad_session = EnvironmentActor {
        user_id: one.user_id,
        session_hash: CredentialHash([42; 32]),
    };
    assert!(matches!(
        store.recoverable(bad_session).await,
        Err(ProjectError::Unauthorized)
    ));
    sqlx::query("UPDATE factory_identity.users SET status='suspended' WHERE id=$1")
        .bind(one.user_id)
        .execute(&sql)
        .await
        .unwrap();
    assert!(matches!(
        store.recoverable(one).await,
        Err(ProjectError::Unauthorized)
    ));
    sqlx::query("UPDATE factory_identity.users SET status='active' WHERE id=$1")
        .bind(one.user_id)
        .execute(&sql)
        .await
        .unwrap();
    // The first bounded batch must remain recoverable even when a 201st import exists.
    sqlx::query("WITH generated AS (SELECT gen_random_uuid() AS id, s FROM generate_series(1,199) s) INSERT INTO factory_identity.projects(id,user_id,repository_id,full_name,cloned_full_name,directory,state,operation_id,environment_id,environment_generation) SELECT id,$1,10000+s,'fixture/repository-' || s,'fixture/repository-' || s,'repository-' || s,'importing',gen_random_uuid(),$2,1 FROM generated")
        .bind(one.user_id).bind(env1).execute(&sql).await.unwrap();
    assert_eq!(
        store.recoverable(one).await.unwrap().len(),
        MAX_RECOVERABLE_IMPORTS
    );
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO factory_identity.projects(id,user_id,repository_id,full_name,cloned_full_name,directory,state,operation_id,environment_id,environment_generation) VALUES($1,$2,20000,'fixture/overflow','fixture/overflow',$3,'importing',$4,$5,1)")
        .bind(id).bind(one.user_id).bind("overflow").bind(Uuid::new_v4()).bind(env1).execute(&sql).await.unwrap();
    let first_batch = store.recoverable(one).await.unwrap();
    assert_eq!(first_batch.len(), MAX_RECOVERABLE_IMPORTS);
    for item in first_batch {
        store
            .recover_stopped(one, env1, 2, observed, item.project_id)
            .await
            .unwrap();
    }
    let second_batch = store.recoverable(one).await.unwrap();
    assert_eq!(second_batch.len(), 1);
    store
        .recover_stopped(one, env1, 2, observed, second_batch[0].project_id)
        .await
        .unwrap();
    assert!(store.recoverable(one).await.unwrap().is_empty());
    assert_eq!(store.recoverable(two).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn provider_metadata_sync_is_atomic_bounded_and_owner_scoped() {
    let (_db, sql) = fixture().await;
    let store = PgProjectStore::new(sql.clone());
    let (one, env1) = actor(&sql).await;
    let (two, env2) = actor(&sql).await;
    let (first, _) = store
        .reserve(one, env1, 1, input(100, "old/first"))
        .await
        .unwrap();
    let (second, _) = store
        .reserve(one, env1, 1, input(300, "old/second"))
        .await
        .unwrap();
    let (other, _) = store
        .reserve(two, env2, 1, input(100, "other/untouched"))
        .await
        .unwrap();
    // Failed reservations still count as existing projects in the import picker.
    store
        .settle(one, env1, 1, second.id, second.operation_id, false)
        .await
        .unwrap();
    let metadata = |repository_id, full_name: &str| RepositoryMetadata {
        repository_id,
        full_name: full_name.to_owned(),
    };
    let page = vec![
        metadata(300, "new/second"),
        metadata(100, "new/first"),
        metadata(400, "unknown/repo"),
        metadata(100, "new/first"),
    ];
    assert_eq!(
        store
            .sync_repository_metadata(one, env1, 1, &page)
            .await
            .unwrap(),
        vec![100, 300]
    );
    let changed = store.get(one, env1, 1, first.id).await.unwrap();
    assert_eq!(changed.full_name, "new/first");
    assert_eq!(changed.cloned_full_name, "old/first");
    assert_eq!(changed.id, first.id);
    assert_eq!(changed.directory, first.directory);
    assert_eq!(
        store.get(one, env1, 1, second.id).await.unwrap().full_name,
        "new/second"
    );
    assert_eq!(
        store.get(two, env2, 1, other.id).await.unwrap().full_name,
        "other/untouched"
    );
    assert_eq!(
        store
            .sync_repository_metadata(two, env2, 1, &page)
            .await
            .unwrap(),
        vec![100]
    );
    assert_eq!(
        store.get(two, env2, 1, other.id).await.unwrap().full_name,
        "new/first"
    );
    assert_eq!(store.list(one, env1, 1, None).await.unwrap().items.len(), 2);
    assert!(
        store
            .sync_repository_metadata(one, env1, 1, &[])
            .await
            .unwrap()
            .is_empty()
    );
    let conflicting = vec![metadata(100, "valid/new"), metadata(100, "valid/other")];
    assert!(matches!(
        store
            .sync_repository_metadata(one, env1, 1, &conflicting)
            .await,
        Err(ProjectError::Invalid)
    ));
    let invalid = vec![metadata(100, "valid/new"), metadata(300, "../unsafe")];
    assert!(matches!(
        store.sync_repository_metadata(one, env1, 1, &invalid).await,
        Err(ProjectError::Invalid)
    ));
    assert!(matches!(
        store
            .sync_repository_metadata(one, env1, 1, &[metadata(0, "valid/name")])
            .await,
        Err(ProjectError::Invalid)
    ));
    let over_limit = vec![metadata(1, "valid/name"); MAX_REPOSITORY_LOOKUP + 1];
    assert!(matches!(
        store
            .sync_repository_metadata(one, env1, 1, &over_limit)
            .await,
        Err(ProjectError::Invalid)
    ));
    assert_eq!(
        store.get(one, env1, 1, first.id).await.unwrap().full_name,
        "new/first"
    );
    assert!(matches!(
        store.sync_repository_metadata(one, env1, 2, &page).await,
        Err(ProjectError::Conflict)
    ));
    sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
        .bind(one.session_hash.0.as_slice())
        .execute(&sql)
        .await
        .unwrap();
    assert!(matches!(
        store.sync_repository_metadata(one, env1, 1, &page).await,
        Err(ProjectError::Unauthorized)
    ));
    assert!(matches!(
        store.sync_repository_metadata(one, env1, 1, &[]).await,
        Err(ProjectError::Unauthorized)
    ));
}

async fn wait_for_lock(sql: &PgPool, query_pattern: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE $1")
                .bind(query_pattern).fetch_one(sql).await.unwrap();
            if blocked > 0 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("operation reaches the expected lock barrier");
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn project_mutations_serialize_with_session_logout_without_deadlock() {
    let (db, sql) = fixture().await;
    let store = PgProjectStore::new(sql.clone());
    let (one, env1) = actor(&sql).await;
    let (two, env2) = actor(&sql).await;

    // Hold environment so reservation locks the session and then blocks at the
    // environment. Logout must wait for that complete transaction to commit.
    let mut environment_lock = sql.begin().await.unwrap();
    sqlx::query("SELECT id FROM factory_identity.environments WHERE id=$1 FOR UPDATE")
        .bind(env1)
        .execute(&mut *environment_lock)
        .await
        .unwrap();
    let first_store = store.clone();
    let first = tokio::spawn(async move {
        first_store
            .reserve(one, env1, 1, input(801, "one/before-logout"))
            .await
    });
    wait_for_lock(
        &sql,
        "%FROM factory_identity.environments WHERE id=%FOR UPDATE%",
    )
    .await;
    let identity_store = db.store();
    let logout = tokio::spawn(async move { identity_store.logout(one.session_hash).await });
    wait_for_lock(
        &sql,
        "%DELETE FROM factory_identity.sessions WHERE token_hash=%",
    )
    .await;
    assert!(!logout.is_finished());
    environment_lock.commit().await.unwrap();
    let (first_project, fresh) = tokio::time::timeout(std::time::Duration::from_secs(5), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(fresh);
    tokio::time::timeout(std::time::Duration::from_secs(5), logout)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        store
            .reserve(one, env1, 1, input(802, "one/after-logout"))
            .await,
        Err(ProjectError::Unauthorized)
    ));
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM factory_identity.projects WHERE id=$1 AND user_id=$2",
    )
    .bind(first_project.id)
    .bind(one.user_id)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(stored, 1);

    // Reverse the race: block the request at the user row, let logout commit,
    // then ensure the request fails once its barrier opens.
    let mut user_lock = sql.begin().await.unwrap();
    sqlx::query("SELECT id FROM factory_identity.users WHERE id=$1 FOR UPDATE")
        .bind(two.user_id)
        .execute(&mut *user_lock)
        .await
        .unwrap();
    let second_store = store.clone();
    let second = tokio::spawn(async move {
        second_store
            .reserve(two, env2, 1, input(803, "two/revoked"))
            .await
    });
    wait_for_lock(
        &sql,
        "%SELECT status FROM factory_identity.users WHERE id=%FOR UPDATE%",
    )
    .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        db.store().logout(two.session_hash),
    )
    .await
    .unwrap()
    .unwrap();
    user_lock.commit().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(5), second)
            .await
            .unwrap()
            .unwrap(),
        Err(ProjectError::Unauthorized)
    ));
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM factory_identity.projects WHERE user_id=$1")
            .bind(two.user_id)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn deletion_fences_runs_and_removes_only_owned_project_history() {
    use factory_core::conversations::{ConversationError, ConversationStore, StartTurn};
    let (db, sql) = fixture().await;
    let store = PgProjectStore::new(sql.clone());
    let conversations = db.conversation_store();
    let (owner, env) = actor(&sql).await;
    let (other, other_env) = actor(&sql).await;
    let (project, _) = store
        .reserve(owner, env, 1, input(825, "owner/deletable"))
        .await
        .unwrap();
    store
        .settle(owner, env, 1, project.id, project.operation_id, true)
        .await
        .unwrap();
    let chat = conversations
        .create_in_project(owner, env, 1, Some(project.id))
        .await
        .unwrap();
    let unrelated = conversations.create(owner, env, 1).await.unwrap();
    let turn = StartTurn {
        conversation_id: chat.id,
        operation_id: Uuid::new_v4(),
        environment_id: env,
        generation: 1,
        model: "fixture".into(),
        input: "test".into(),
    };
    let (run, _) = conversations.begin_turn(owner, turn.clone()).await.unwrap();
    assert!(matches!(
        store.begin_delete(owner, env, 1, project.id).await,
        Err(ProjectError::InUse)
    ));
    assert!(matches!(
        store.begin_delete(other, other_env, 1, project.id).await,
        Err(ProjectError::NotFound)
    ));
    sqlx::query("UPDATE factory_identity.conversation_runs SET status='completed' WHERE id=$1")
        .bind(run.id)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(
        store
            .begin_delete(owner, env, 1, project.id)
            .await
            .unwrap()
            .state,
        ProjectState::Deleting
    );
    assert_eq!(
        store
            .begin_delete(owner, env, 1, project.id)
            .await
            .unwrap()
            .state,
        ProjectState::Deleting
    );
    let mut new_turn = turn;
    new_turn.operation_id = Uuid::new_v4();
    assert!(matches!(
        conversations.begin_turn(owner, new_turn).await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        conversations
            .create_in_project(owner, env, 1, Some(project.id))
            .await,
        Err(ConversationError::NotFound)
    ));
    store
        .finish_delete(owner, env, 1, project.id)
        .await
        .unwrap();
    assert!(matches!(
        store.get(owner, env, 1, project.id).await,
        Err(ProjectError::NotFound)
    ));
    assert!(matches!(
        conversations.get(owner, env, 1, chat.id).await,
        Err(ConversationError::NotFound)
    ));
    assert!(conversations.get(owner, env, 1, unrelated.id).await.is_ok());
    let messages: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM factory_identity.conversation_messages WHERE run_id=$1",
    )
    .bind(run.id)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(messages, 0);
    sql.close().await;
}
