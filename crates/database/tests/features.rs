use factory_core::{
    conversations::{ConversationError, ConversationStore, MessageRole},
    environments::{Environment, EnvironmentActor, EnvironmentStore},
    features::*,
    identity::CredentialHash,
    projects::{ProjectError, ProjectStore},
};
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
    let database = IdentityDatabase::connect(&target).await.unwrap();
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
    (database, sql)
}

async fn member(
    database: &IdentityDatabase,
    sql: &PgPool,
) -> (EnvironmentActor, Environment, Uuid) {
    let user_id = Uuid::new_v4();
    let environment_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let mut session_hash = [0; 32];
    session_hash[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    session_hash[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Feature test','member','active',clock_timestamp())")
        .bind(user_id).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')")
        .bind(session_hash.as_slice()).bind(user_id).bind("c".repeat(32)).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,observed_generation,volume_name,container_name,image_id,policy_version) VALUES($1,$2,$3,'running','running',1,$4,$5,'image','1')")
        .bind(environment_id).bind(user_id).bind(Uuid::new_v4()).bind(format!("v-{environment_id}")).bind(format!("c-{environment_id}")).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.projects(id,user_id,repository_id,full_name,cloned_full_name,directory,state,operation_id,environment_id,environment_generation) VALUES($1,$2,1,'fixture/repo','fixture/repo','repo','ready',$3,$4,1)")
        .bind(project_id).bind(user_id).bind(Uuid::new_v4()).bind(environment_id).execute(sql).await.unwrap();
    let actor = EnvironmentActor {
        user_id,
        session_hash: CredentialHash(session_hash),
    };
    let environment = database
        .environment_store()
        .get(actor)
        .await
        .unwrap()
        .unwrap();
    (actor, environment, project_id)
}

fn request(project_id: Uuid) -> NewFeature {
    NewFeature {
        operation_id: Uuid::new_v4(),
        project_id,
        provider: "codex".into(),
        workflow_revision: "build-feature/1".into(),
        title: "Fixture feature".into(),
        opener: "What would you like to build?".into(),
    }
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn feature_creation_and_approval_are_atomic_owned_and_replayable() {
    let (database, sql) = fixture().await;
    let (actor, environment, project_id) = member(&database, &sql).await;
    let (other, _, _) = member(&database, &sql).await;
    let store = database.feature_store();
    let input = request(project_id);
    let (first, replay) = tokio::join!(
        store.create(actor, &environment, input.clone()),
        store.create(actor, &environment, input.clone())
    );
    let first = first.unwrap();
    assert_eq!(first, replay.unwrap());
    let id = first.feature.id;
    let conversation_id = first.feature.conversation_id;

    let history = database
        .conversation_store()
        .history(
            actor,
            environment.id,
            environment.generation,
            conversation_id,
        )
        .await
        .unwrap();
    assert_eq!(history.conversation.project_id, Some(project_id));
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].role, MessageRole::Assistant);
    assert_eq!(history.messages[0].content, input.opener);
    let conversations = database.conversation_store();
    assert!(
        conversations
            .list(actor, environment.id, environment.generation, None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(matches!(
        conversations
            .list(
                actor,
                environment.id,
                environment.generation,
                Some(conversation_id)
            )
            .await,
        Err(ConversationError::NotFound)
    ));
    assert_eq!(
        store
            .list(actor, Some(project_id), None)
            .await
            .unwrap()
            .items,
        vec![first.feature]
    );
    assert_eq!(store.get(other, id).await, Err(FeatureError::NotFound));
    assert_eq!(
        store
            .get_by_conversation(other, conversation_id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.list(other, None, Some(id)).await,
        Err(FeatureError::NotFound)
    );

    let mut changed = input;
    changed.title = "Different request".into();
    assert_eq!(
        store.create(actor, &environment, changed).await,
        Err(FeatureError::Conflict)
    );

    let key = RevisionKey {
        id: Uuid::new_v4(),
        digest: "a".repeat(64),
    };
    store
        .apply(
            actor,
            &environment,
            id,
            Uuid::new_v4(),
            FeatureCommand::ProposeRevision(GateRevision {
                key: key.clone(),
                content: GateContent::Scope {
                    brief: "Add an owned feature".into(),
                },
                approval: None,
            }),
        )
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    let command = FeatureCommand::ApproveRevision(key.clone());
    let (approved, replay) = tokio::join!(
        store.apply(actor, &environment, id, operation, command.clone()),
        store.apply(actor, &environment, id, operation, command),
    );
    let (approved, replay) = (approved.unwrap(), replay.unwrap());
    assert_ne!(approved.fresh, replay.fresh);
    assert_eq!(approved.state, replay.state);
    assert_eq!(approved.state.feature.stage, FeatureStage::Architecture);
    assert_eq!(
        approved
            .state
            .current_scope()
            .unwrap()
            .approval
            .unwrap()
            .user_id,
        actor.user_id
    );
    assert_eq!(
        store
            .apply(
                actor,
                &environment,
                id,
                operation,
                FeatureCommand::ApproveRevision(RevisionKey {
                    id: key.id,
                    digest: "b".repeat(64)
                })
            )
            .await,
        Err(FeatureError::Conflict)
    );
    sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
        .bind(actor.user_id)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(store.get(actor, id).await, Err(FeatureError::Unauthorized));
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn attempts_fence_project_deletion_and_recovery_checks_the_generation() {
    let (database, sql) = fixture().await;
    let (actor, environment, project_id) = member(&database, &sql).await;
    let store = database.feature_store();
    let feature = store
        .create(actor, &environment, request(project_id))
        .await
        .unwrap()
        .feature;
    let attempt_id = Uuid::new_v4();
    let command = FeatureCommand::ClaimAttempt(AttemptClaim {
        task: FeatureTask {
            id: Uuid::new_v4(),
            title: "Discuss feature".into(),
            instructions: "Gather requirements".into(),
            role: TaskRole::Coordinator,
            paths: Vec::new(),
            depends_on: Vec::new(),
            checks: Vec::new(),
        },
        revision: None,
        workspace: TaskWorkspace {
            feature_id: feature.id,
            attempt_id,
            base_oid: "a".repeat(40),
            access: WorkspaceAccess::ReadOnly,
        },
        snapshot_id: None,
        model: "fixture".into(),
        effort: None,
        skill_revision: "build-feature/1".into(),
        retry_of: None,
    });
    let operation = Uuid::new_v4();
    assert!(
        store
            .apply(actor, &environment, feature.id, operation, command.clone())
            .await
            .unwrap()
            .fresh
    );
    assert!(
        !store
            .apply(actor, &environment, feature.id, operation, command)
            .await
            .unwrap()
            .fresh
    );
    assert!(
        database
            .conversation_store()
            .has_open_runs(actor)
            .await
            .unwrap()
    );
    assert!(matches!(
        database
            .project_store()
            .begin_delete(actor, environment.id, 1, project_id)
            .await,
        Err(ProjectError::InUse)
    ));
    let record = FeatureCommand::RecordAttempt {
        attempt_id,
        update: AttemptUpdate::LaunchSettled {
            process_receipt: Some("worker-1".into()),
        },
    };
    store
        .apply(
            actor,
            &environment,
            feature.id,
            Uuid::new_v4(),
            record.clone(),
        )
        .await
        .unwrap();
    let stopped = database
        .environment_store()
        .stop(actor)
        .await
        .unwrap()
        .unwrap();
    let recovery = FeatureCommand::RecoverAttempt {
        attempt_id,
        launch_generation: environment.generation,
    };
    assert_eq!(
        store
            .apply(
                actor,
                &stopped,
                feature.id,
                Uuid::new_v4(),
                recovery.clone()
            )
            .await,
        Err(FeatureError::Conflict)
    );
    assert_eq!(
        store
            .apply(actor, &environment, feature.id, Uuid::new_v4(), record)
            .await,
        Err(FeatureError::Conflict)
    );
    sqlx::query("UPDATE factory_identity.environments SET observed_state='stopped',observed_generation=$2 WHERE id=$1")
        .bind(environment.id).bind(stopped.generation).execute(&sql).await.unwrap();
    let recovered = store
        .apply(actor, &stopped, feature.id, Uuid::new_v4(), recovery)
        .await
        .unwrap();
    assert_eq!(
        recovered.state.attempts[0].status,
        AttemptStatus::Interrupted
    );
    assert!(
        !database
            .conversation_store()
            .has_open_runs(actor)
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .get_by_conversation(actor, feature.conversation_id)
            .await
            .unwrap(),
        Some(recovered.state)
    );
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn upgrading_v4_preserves_existing_project_conversations() {
    let (database, sql) = fixture().await;
    let (actor, environment, project_id) = member(&database, &sql).await;
    let conversation = database
        .conversation_store()
        .create_in_project(
            actor,
            environment.id,
            environment.generation,
            Some(project_id),
        )
        .await
        .unwrap();
    let upgrade = include_str!("../schema-upgrades/0005-features.sql");
    let create = include_str!("../schema.sql");
    let marker = "-- Product workflow records";
    let feature_ddl = upgrade
        .split_once(marker)
        .unwrap()
        .1
        .split_once("ALTER TABLE factory_identity.schema_info")
        .unwrap()
        .0;
    assert!(
        create.contains(feature_ddl),
        "fresh and upgraded schemas must agree"
    );
    // Roll back the entire scenario so the shared disposable fixture stays v5.
    let mut tx = sql.begin().await.unwrap();
    sqlx::raw_sql(
        "DROP TABLE factory_identity.feature_operations,
                    factory_identity.feature_deliveries,
                    factory_identity.feature_reviews,
                    factory_identity.feature_attempts,
                    factory_identity.feature_revisions,
                    factory_identity.features;
         ALTER TABLE factory_identity.schema_info DROP CONSTRAINT schema_info_version_check;
         UPDATE factory_identity.schema_info SET version=4;
         ALTER TABLE factory_identity.schema_info ADD CONSTRAINT schema_info_version_check CHECK (version=4);",
    ).execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(upgrade).execute(&mut *tx).await.unwrap();
    let (version, owner, project, feature_table): (i32, Uuid, Uuid, bool) = sqlx::query_as(
        "SELECT s.version,c.user_id,c.project_id,to_regclass('factory_identity.features') IS NOT NULL
         FROM factory_identity.schema_info s CROSS JOIN factory_identity.conversations c WHERE c.id=$1",
    ).bind(conversation.id).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(
        (version, owner, project, feature_table),
        (5, actor.user_id, project_id, true)
    );
    tx.rollback().await.unwrap();
}
