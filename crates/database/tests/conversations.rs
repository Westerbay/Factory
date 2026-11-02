use factory_core::{conversations::*, environments::*, identity::CredentialHash};
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
async fn actor(sql: &PgPool, _marker: u8) -> (EnvironmentActor, Uuid) {
    let user = Uuid::new_v4();
    let env = Uuid::new_v4();
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let hash = CredentialHash(bytes);
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Conversation test','member','active',clock_timestamp())").bind(user).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')").bind(hash.0.as_slice()).bind(user).bind("c".repeat(32)).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,volume_name,container_name,image_id,policy_version) VALUES($1,$2,$3,'running','running',$4,$5,'image','1')").bind(env).bind(user).bind(Uuid::new_v4()).bind(format!("v-{env}")).bind(format!("c-{env}")).execute(sql).await.unwrap();
    (
        EnvironmentActor {
            user_id: user,
            session_hash: hash,
        },
        env,
    )
}
#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn conversations_are_owned_idempotent_and_block_uncertain_runs() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let (one, env1) = actor(&sql, 201).await;
    let (two, env2) = actor(&sql, 202).await;
    let conversation = store.create(one, env1, 1).await.unwrap();
    assert!(matches!(
        store.history(two, env2, 1, conversation.id).await,
        Err(ConversationError::NotFound)
    ));
    assert!(matches!(
        store.history(one, env1, 2, conversation.id).await,
        Err(ConversationError::Conflict)
    ));
    let input = StartTurn {
        conversation_id: conversation.id,
        operation_id: Uuid::new_v4(),
        environment_id: env1,
        generation: 1,
        model: "test-model".into(),
        input: "hello".into(),
    };
    assert!(!store.has_open_runs(one).await.unwrap());
    assert_eq!(store.find_turn(one, input.clone()).await.unwrap(), None);
    let (run, new) = store.begin_turn(one, input.clone()).await.unwrap();
    assert!(store.has_open_runs(one).await.unwrap());
    assert!(!store.has_open_runs(two).await.unwrap());
    assert!(new);
    let (repeated, new) = store.begin_turn(one, input.clone()).await.unwrap();
    assert!(!new);
    assert_eq!(run, repeated);
    assert_eq!(
        store.find_turn(one, input.clone()).await.unwrap(),
        Some(run.clone())
    );
    assert!(matches!(
        store.find_turn(two, input.clone()).await,
        Err(ConversationError::Conflict)
    ));
    let mut other_user_turn = input.clone();
    other_user_turn.environment_id = env2;
    assert!(matches!(
        store.find_turn(two, other_user_turn).await,
        Err(ConversationError::NotFound)
    ));
    let revoked = EnvironmentActor {
        user_id: one.user_id,
        session_hash: CredentialHash([0; 32]),
    };
    assert!(matches!(
        store.find_turn(revoked, input.clone()).await,
        Err(ConversationError::Unauthorized)
    ));
    let mut different = input.clone();
    different.input = "different".into();
    assert!(matches!(
        store.find_turn(one, different.clone()).await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        store.begin_turn(one, different).await,
        Err(ConversationError::Conflict)
    ));
    let (first, fresh) = store
        .append_assistant(one, env1, 1, run.id, "provider-item-a", "First")
        .await
        .unwrap();
    assert!(fresh);
    let (replayed, fresh) = store
        .append_assistant(one, env1, 1, run.id, "provider-item-a", "First")
        .await
        .unwrap();
    assert!(!fresh);
    assert_eq!(first, replayed);
    assert!(matches!(
        store
            .append_assistant(one, env1, 1, run.id, "provider-item-a", "Changed")
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(
        store
            .append_assistant(one, env1, 1, run.id, "provider-item-b", "Second")
            .await
            .unwrap()
            .1
    );
    store
        .set_thread(one, env1, 1, run.id, "thread-1", None)
        .await
        .unwrap();
    assert!(matches!(
        store
            .set_thread(one, env1, 1, run.id, "thread-2", None)
            .await,
        Err(ConversationError::Conflict)
    ));
    store
        .set_thread(one, env1, 1, run.id, "thread-1", None)
        .await
        .unwrap();
    store
        .set_thread(one, env1, 1, run.id, "combined-threads", Some("thread-1"))
        .await
        .unwrap();
    assert!(matches!(
        store
            .set_thread(one, env1, 1, run.id, "stale-update", Some("thread-1"))
            .await,
        Err(ConversationError::Conflict)
    ));
    let request = PendingRequest {
        id: Uuid::new_v4(),
        run_id: run.id,
        kind: RequestKind::Permission,
        state: RequestState::Pending,
        details: "Allow command?".into(),
    };
    store
        .add_request(one, env1, 1, request.clone(), "private-provider-id")
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    let claimed = store
        .claim_response(one, env1, 1, request.id, operation, "deny")
        .await
        .unwrap();
    assert!(claimed.fresh);
    assert_eq!(
        store
            .history(one, env1, 1, conversation.id)
            .await
            .unwrap()
            .requests[0]
            .state,
        RequestState::Claimed
    );
    assert_eq!(claimed.provider_reference, "private-provider-id");
    let repeated = store
        .claim_response(one, env1, 1, request.id, operation, "deny")
        .await
        .unwrap();
    assert!(!repeated.fresh);
    assert_eq!(repeated.state, ResponseState::Claimed);
    assert!(matches!(
        store
            .claim_response(one, env1, 1, request.id, operation, "allow")
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        store
            .claim_response(two, env2, 1, request.id, operation, "deny")
            .await,
        Err(ConversationError::NotFound)
    ));
    store
        .record_response(
            one,
            env1,
            1,
            request.id,
            operation,
            ResponseState::Uncertain,
        )
        .await
        .unwrap();
    let repeated = store
        .claim_response(one, env1, 1, request.id, operation, "deny")
        .await
        .unwrap();
    assert!(!repeated.fresh);
    assert_eq!(repeated.state, ResponseState::Uncertain);
    assert_eq!(
        store
            .history(one, env1, 1, conversation.id)
            .await
            .unwrap()
            .requests[0]
            .state,
        RequestState::Uncertain
    );
    // Only a later runtime-correlated serverRequest/resolved allows this transition.
    store
        .record_response(
            one,
            env1,
            1,
            request.id,
            operation,
            ResponseState::Delivered,
        )
        .await
        .unwrap();
    let acknowledged = store
        .claim_response(one, env1, 1, request.id, operation, "deny")
        .await
        .unwrap();
    assert!(!acknowledged.fresh);
    assert_eq!(acknowledged.state, ResponseState::Delivered);
    store
        .finish_turn(one, env1, 1, run.id, RunStatus::Uncertain)
        .await
        .unwrap();
    let mut next = input;
    next.operation_id = Uuid::new_v4();
    assert!(matches!(
        store.begin_turn(one, next.clone()).await,
        Err(ConversationError::Conflict)
    ));
    store.settle_run_launch(run.id, env1, 1).await.unwrap();
    store
        .finish_turn(one, env1, 1, run.id, RunStatus::Interrupted)
        .await
        .unwrap();
    let (recovery_run, _) = store.begin_turn(one, next).await.unwrap();
    store
        .settle_run_launch(recovery_run.id, env1, 1)
        .await
        .unwrap();
    let history = store.history(one, env1, 1, conversation.id).await.unwrap();
    assert!(!history.runs_truncated && !history.messages_truncated && !history.requests_truncated);
    assert_eq!(history.messages.len(), 4);
    assert_eq!(
        history
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["hello", "First", "Second", "hello"]
    );
    // A stale "stopped" state survives an intent change but cannot prove this
    // generation was inspected; only a new supervisor observation can do so.
    sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped', generation=2, observed_state='stopped', observed_generation=1, observed_at=clock_timestamp()-interval '1 minute' WHERE id=$1").bind(env1).execute(&sql).await.unwrap();
    let stale: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT observed_at FROM factory_identity.environments WHERE id=$1")
            .bind(env1)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert!(matches!(
        store.recoverable_stopped(one, env1, 2, stale).await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        store
            .recover_stopped(one, env1, 2, stale, recovery_run.id)
            .await,
        Err(ConversationError::Conflict)
    ));
    let environments = db.environment_store();
    assert!(
        environments
            .record_observation(
                env1,
                2,
                EnvironmentObservation {
                    state: ObservedState::Stopped,
                    error: None,
                    volume_confirmed: true
                },
                15
            )
            .await
            .unwrap()
    );
    let fresh: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT observed_at FROM factory_identity.environments WHERE id=$1")
            .bind(env1)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert!(matches!(
        store
            .recover_stopped(one, env1, 2, stale, recovery_run.id)
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        store
            .recover_stopped(two, env1, 2, fresh, recovery_run.id)
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        store.recoverable_stopped(two, env1, 2, fresh).await,
        Err(ConversationError::Conflict)
    ));
    assert_eq!(
        store
            .recoverable_stopped(one, env1, 2, fresh)
            .await
            .unwrap(),
        vec![RecoverableRun {
            conversation_id: conversation.id,
            run_id: recovery_run.id,
            launch_settled: true,
        }]
    );
    store
        .recover_stopped(one, env1, 2, fresh, recovery_run.id)
        .await
        .unwrap();
    assert!(!store.has_open_runs(one).await.unwrap());
    assert!(
        store
            .recoverable_stopped(one, env1, 2, fresh)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE factory_identity.environments SET desired_state='running', generation=3, observed_state='running' WHERE id=$1").bind(env1).execute(&sql).await.unwrap();
    let history = store.history(one, env1, 3, conversation.id).await.unwrap();
    assert_eq!(history.runs.last().unwrap().status, RunStatus::Interrupted);
    sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
        .bind(one.user_id)
        .execute(&sql)
        .await
        .unwrap();
    assert!(matches!(
        store.history(one, env1, 3, conversation.id).await,
        Err(ConversationError::Unauthorized)
    ));
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn bounded_history_keeps_active_work_and_discloses_older_records() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let (actor, env) = actor(&sql, 203).await;
    let conversation = store.create(actor, env, 1).await.unwrap();
    // Older durable rows exceed both history limits; no provider or real account is used.
    sqlx::query("INSERT INTO factory_identity.conversation_runs(id,conversation_id,operation_id,environment_id,environment_generation,model,input,status,created_at) SELECT md5($1::text || n::text)::uuid,$1,md5('operation' || $1::text || n::text)::uuid,$2,1,'test-model','old','completed',clock_timestamp()-n*interval '1 second' FROM generate_series(1,1002) n")
        .bind(conversation.id).bind(env).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.conversation_messages(id,run_id,role,content,ordinal) SELECT md5(r.id::text || n::text)::uuid,r.id,CASE WHEN n=0 THEN 'user' ELSE 'assistant' END,'old',n FROM factory_identity.conversation_runs r CROSS JOIN generate_series(0,1) n WHERE r.conversation_id=$1")
        .bind(conversation.id).execute(&sql).await.unwrap();
    let (active, _) = store
        .begin_turn(
            actor,
            StartTurn {
                conversation_id: conversation.id,
                operation_id: Uuid::new_v4(),
                environment_id: env,
                generation: 1,
                model: "test-model".into(),
                input: "current".into(),
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO factory_identity.conversation_requests(id,run_id,kind,details,provider_reference) SELECT md5($1::text || n::text)::uuid,$1,'question','fixture',n::text FROM generate_series(1,101) n")
        .bind(active.id).execute(&sql).await.unwrap();
    let history = store.history(actor, env, 1, conversation.id).await.unwrap();
    assert!(history.runs_truncated && history.messages_truncated && history.requests_truncated);
    assert_eq!(history.runs.len(), 1000);
    assert_eq!(history.messages.len(), 2000);
    assert_eq!(history.requests.len(), 100);
    assert!(
        history
            .runs
            .iter()
            .any(|run| run.id == active.id && run.status == RunStatus::Active)
    );
    assert!(
        history
            .messages
            .iter()
            .any(|message| message.run_id == active.id && message.content == "current")
    );
    assert!(history.runs.last().is_some_and(|run| run.id == active.id));
    assert!(
        history
            .messages
            .last()
            .is_some_and(|message| message.run_id == active.id)
    );
    store.settle_run_launch(active.id, env, 1).await.unwrap();
    store
        .finish_turn(actor, env, 1, active.id, RunStatus::Interrupted)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn conversation_pages_are_stable_bounded_and_user_owned() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let (one, env1) = actor(&sql, 204).await;
    let (two, env2) = actor(&sql, 205).await;
    // Equal timestamps exercise the UUID tie-breaker in keyset pagination.
    sqlx::query("INSERT INTO factory_identity.conversations(id,user_id,created_at) SELECT md5($1::text || n::text)::uuid,$1,'2020-01-01T00:00:00Z' FROM generate_series(1,205) n")
        .bind(one.user_id).execute(&sql).await.unwrap();
    let other = store.create(two, env2, 1).await.unwrap();
    let first = store.list(one, env1, 1, None).await.unwrap();
    assert_eq!(first.items.len(), 200);
    let cursor = first.next_cursor.unwrap();
    let next = store.list(one, env1, 1, Some(cursor)).await.unwrap();
    assert_eq!(next.items.len(), 5);
    assert_eq!(next.next_cursor, None);
    let ids: std::collections::HashSet<_> = first
        .items
        .iter()
        .chain(&next.items)
        .map(|c| c.id)
        .collect();
    assert_eq!(ids.len(), 205);
    assert!(!ids.contains(&other.id));
    assert!(matches!(
        store.list(one, env1, 1, Some(other.id)).await,
        Err(ConversationError::NotFound)
    ));
    assert!(matches!(
        store.list(two, env2, 1, Some(cursor)).await,
        Err(ConversationError::NotFound)
    ));
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn model_probe_leases_survive_reconnect_and_recover_only_after_verified_stop() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let mut owners = Vec::new();
    for i in 0..9 {
        owners.push(actor(&sql, i).await);
    }
    let mut leases = Vec::new();
    for (a, e) in owners.iter().take(8) {
        let lease = Uuid::new_v4();
        store.reserve_model_probe(*a, *e, 1, lease).await.unwrap();
        store.settle_model_probe_launch(lease, *e, 1).await.unwrap();
        leases.push(lease);
    }
    assert!(store.has_open_runs(owners[0].0).await.unwrap());
    assert!(!store.has_open_runs(owners[8].0).await.unwrap());
    assert!(matches!(
        store
            .reserve_model_probe(owners[0].0, owners[0].1, 1, Uuid::new_v4())
            .await,
        Err(ConversationError::Conflict)
    ));
    // A new store handle sees the same reservation; an API restart cannot erase it.
    let reopened = db.conversation_store();
    assert!(matches!(
        reopened
            .reserve_model_probe(owners[0].0, owners[0].1, 1, Uuid::new_v4())
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        reopened.release_model_probe(owners[1].0, leases[0]).await,
        Err(ConversationError::Conflict)
    ));
    let (a, e) = owners[0];
    let when: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("UPDATE factory_identity.environments SET desired_state='stopped', observed_state='stopped', generation=2, observed_generation=1, observed_at=clock_timestamp() WHERE id=$1 RETURNING observed_at").bind(e).fetch_one(&sql).await.unwrap();
    assert!(matches!(
        reopened.recover_model_probe_stopped(a, e, 2, when).await,
        Err(ConversationError::Conflict)
    ));
    let when: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("UPDATE factory_identity.environments SET observed_generation=2, observed_at=clock_timestamp() WHERE id=$1 RETURNING observed_at").bind(e).fetch_one(&sql).await.unwrap();
    assert!(matches!(
        reopened
            .recover_model_probe_stopped(a, e, 2, when - chrono::Duration::seconds(1))
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        reopened
            .recover_model_probe_stopped(owners[1].0, e, 2, when)
            .await,
        Err(ConversationError::Conflict)
    ));
    assert!(
        reopened
            .recover_model_probe_stopped(a, e, 2, when)
            .await
            .unwrap()
    );
    assert!(!reopened.has_open_runs(a).await.unwrap());
    assert!(
        !reopened
            .recover_model_probe_stopped(a, e, 2, when)
            .await
            .unwrap()
    );
    let new_lease = Uuid::new_v4();
    reopened
        .reserve_model_probe(owners[8].0, owners[8].1, 1, new_lease)
        .await
        .unwrap();
    reopened
        .settle_model_probe_launch(new_lease, owners[8].1, 1)
        .await
        .unwrap();
    reopened
        .release_model_probe(owners[8].0, new_lease)
        .await
        .unwrap();
    for ((a, _), id) in owners.iter().skip(1).take(7).zip(leases.iter().skip(1)) {
        reopened.release_model_probe(*a, *id).await.unwrap();
    }
    // Independent users may reserve concurrently without a global cap.
    let mut racers = Vec::new();
    for i in 0..9 {
        racers.push(actor(&sql, i).await);
    }
    let racers_by_user = racers
        .iter()
        .map(|(a, e)| (a.user_id, *e))
        .collect::<std::collections::HashMap<_, _>>();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(9));
    let mut tasks = Vec::new();
    for (actor, env) in racers {
        let store = reopened.clone();
        let barrier = barrier.clone();
        let lease = Uuid::new_v4();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            (
                actor,
                lease,
                store.reserve_model_probe(actor, env, 1, lease).await,
            )
        }));
    }
    let mut winners = Vec::new();
    let mut unavailable = 0;
    for task in tasks {
        let (actor, lease, result) = task.await.unwrap();
        match result {
            Ok(()) => winners.push((actor, lease)),
            Err(ConversationError::Unavailable) => unavailable += 1,
            other => panic!("unexpected reservation result: {other:?}"),
        }
    }
    assert_eq!(winners.len(), 9);
    assert_eq!(unavailable, 0);
    for (actor, lease) in winners {
        let env = racers_by_user[&actor.user_id];
        reopened
            .settle_model_probe_launch(lease, env, 1)
            .await
            .unwrap();
        reopened.release_model_probe(actor, lease).await.unwrap();
    }
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn supervisor_releases_suspended_users_probe_only_after_fresh_stopped_generation() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let env_store = db.environment_store();
    let (a, env) = actor(&sql, 88).await;
    let lease = Uuid::new_v4();
    store.reserve_model_probe(a, env, 1, lease).await.unwrap();
    store
        .settle_model_probe_launch(lease, env, 1)
        .await
        .unwrap();
    sqlx::query("UPDATE factory_identity.users SET status='suspended' WHERE id=$1")
        .bind(a.user_id)
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
        .bind(a.user_id)
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped' WHERE id=$1")
        .bind(env)
        .execute(&sql)
        .await
        .unwrap();
    let count = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM factory_identity.model_probe_leases WHERE id=$1",
        )
        .bind(lease)
        .fetch_one(&sql)
        .await
        .unwrap()
    };
    let observation = |state, error| EnvironmentObservation {
        state,
        error,
        volume_confirmed: true,
    };
    // A stop intent and same-generation observation cannot prove old ownership ended.
    assert!(
        env_store
            .record_observation(env, 1, observation(ObservedState::Stopped, None), 1)
            .await
            .unwrap()
    );
    assert_eq!(count().await, 1);
    sqlx::query("UPDATE factory_identity.environments SET generation=2 WHERE id=$1")
        .bind(env)
        .execute(&sql)
        .await
        .unwrap();
    // A stale callback, an intermediate state and an errored result cannot release it.
    assert!(
        !env_store
            .record_observation(env, 1, observation(ObservedState::Stopped, None), 1)
            .await
            .unwrap()
    );
    assert_eq!(count().await, 1);
    assert!(
        env_store
            .record_observation(env, 2, observation(ObservedState::Stopping, None), 1)
            .await
            .unwrap()
    );
    assert_eq!(count().await, 1);
    assert!(
        env_store
            .record_observation(
                env,
                2,
                observation(ObservedState::Stopped, Some(EnvironmentError::Unavailable)),
                1
            )
            .await
            .unwrap()
    );
    assert_eq!(count().await, 1);
    // A new store handle represents the supervisor after restart; it must inspect
    // and record the new generation, not merely trust the prior stopped row.
    let reopened = db.environment_store();
    assert_eq!(count().await, 1);
    assert!(
        reopened
            .record_observation(env, 2, observation(ObservedState::Stopped, None), 1)
            .await
            .unwrap()
    );
    assert_eq!(count().await, 0);
    let status: String =
        sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1")
            .bind(a.user_id)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(status, "suspended");
    let sessions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM factory_identity.sessions WHERE user_id=$1")
            .bind(a.user_id)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(sessions, 0);
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn unsettled_launches_survive_stop_restart_and_suspension_until_settled() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let environments = db.environment_store();
    let (actor, env) = actor(&sql, 90).await;
    let lease = Uuid::new_v4();
    store
        .reserve_model_probe(actor, env, 1, lease)
        .await
        .unwrap();
    let c = store.create(actor, env, 1).await.unwrap();
    let (run, _) = store
        .begin_turn(
            actor,
            StartTurn {
                conversation_id: c.id,
                operation_id: Uuid::new_v4(),
                environment_id: env,
                generation: 1,
                model: "model".into(),
                input: "hello".into(),
            },
        )
        .await
        .unwrap();
    // Simulate the provider request paused after durable reservation, followed by
    // a stopped observation. Neither durable reservation may be released yet.
    sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped', generation=2 WHERE id=$1").bind(env).execute(&sql).await.unwrap();
    let obs = EnvironmentObservation {
        state: ObservedState::Stopped,
        error: None,
        volume_confirmed: true,
    };
    assert!(
        environments
            .record_observation(env, 2, obs, 1)
            .await
            .unwrap()
    );
    let when: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT observed_at FROM factory_identity.environments WHERE id=$1")
            .bind(env)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert!(matches!(
        store.recover_model_probe_stopped(actor, env, 2, when).await,
        Err(ConversationError::Conflict)
    ));
    assert!(matches!(
        store.recover_stopped(actor, env, 2, when, run.id).await,
        Err(ConversationError::Conflict)
    ));
    let listed = store
        .recoverable_stopped(actor, env, 2, when)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].launch_settled);

    let reopened = db.conversation_store();
    assert!(reopened.has_open_runs(actor).await.unwrap());
    // Restarting the same container must not free the old launch; if it later
    // completes, only a subsequent stop may recover the now-settled launch.
    sqlx::query("UPDATE factory_identity.environments SET desired_state='running', observed_state='running', generation=3, observed_generation=3, observed_at=clock_timestamp() WHERE id=$1").bind(env).execute(&sql).await.unwrap();
    assert!(matches!(
        reopened
            .reserve_model_probe(actor, env, 3, Uuid::new_v4())
            .await,
        Err(ConversationError::Conflict)
    ));
    reopened
        .settle_model_probe_launch(lease, env, 1)
        .await
        .unwrap();
    reopened.settle_run_launch(run.id, env, 1).await.unwrap();
    assert!(matches!(
        reopened.recover_stopped(actor, env, 2, when, run.id).await,
        Err(ConversationError::Conflict)
    ));
    sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped', generation=4 WHERE id=$1").bind(env).execute(&sql).await.unwrap();
    assert!(
        environments
            .record_observation(env, 4, obs, 1)
            .await
            .unwrap()
    );
    let current: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT observed_at FROM factory_identity.environments WHERE id=$1")
            .bind(env)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert!(
        !reopened
            .recover_model_probe_stopped(actor, env, 4, current)
            .await
            .unwrap()
    ); // supervisor already recovered it
    reopened
        .recover_stopped(actor, env, 4, current, run.id)
        .await
        .unwrap();
    assert!(!reopened.has_open_runs(actor).await.unwrap());
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn independent_runs_are_not_globally_capped_but_each_conversation_remains_exclusive() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let mut participants = Vec::new();
    for i in 0..40 {
        let (actor, env) = actor(&sql, i).await;
        let conversation = store.create(actor, env, 1).await.unwrap();
        participants.push((actor, env, conversation.id));
    }
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(40));
    let mut tasks = Vec::new();
    for (actor, env, conversation_id) in participants {
        let store = store.clone();
        let barrier = barrier.clone();
        let input = StartTurn {
            conversation_id,
            operation_id: Uuid::new_v4(),
            environment_id: env,
            generation: 1,
            model: "model".into(),
            input: "hello".into(),
        };
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let result = store.begin_turn(actor, input.clone()).await;
            (actor, input, result)
        }));
    }
    let mut accepted = Vec::new();
    for task in tasks {
        let (actor, input, result) = task.await.unwrap();
        let (run, fresh) = result.unwrap();
        assert!(fresh);
        accepted.push((actor, input, run));
    }
    assert_eq!(accepted.len(), 40);
    let (owner, input, run) = &accepted[0];
    // Replaying the same operation never starts another run.
    let (replayed, fresh) = store.begin_turn(*owner, input.clone()).await.unwrap();
    assert!(!fresh);
    assert_eq!(replayed.id, run.id);
    let mut different = input.clone();
    different.operation_id = Uuid::new_v4();
    assert!(matches!(
        store.begin_turn(*owner, different.clone()).await,
        Err(ConversationError::Conflict)
    ));
    store
        .finish_turn(
            *owner,
            input.environment_id,
            1,
            run.id,
            RunStatus::Uncertain,
        )
        .await
        .unwrap();
    assert!(matches!(
        store.begin_turn(*owner, different).await,
        Err(ConversationError::Conflict)
    ));
    // Cleanup must settle the original launch before it can be marked interrupted.
    store
        .settle_run_launch(run.id, input.environment_id, 1)
        .await
        .unwrap();
    store
        .finish_turn(
            *owner,
            input.environment_id,
            1,
            run.id,
            RunStatus::Interrupted,
        )
        .await
        .unwrap();
    for (actor, input, run) in accepted.iter().skip(1) {
        store
            .settle_run_launch(run.id, input.environment_id, 1)
            .await
            .unwrap();
        store
            .finish_turn(
                *actor,
                input.environment_id,
                1,
                run.id,
                RunStatus::Interrupted,
            )
            .await
            .unwrap();
    }
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn suspended_owner_cannot_release_unsettled_launch_but_supervisor_can_after_settlement() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let envs = db.environment_store();
    let (a, env) = actor(&sql, 91).await;
    let lease = Uuid::new_v4();
    store.reserve_model_probe(a, env, 1, lease).await.unwrap();
    sqlx::query("UPDATE factory_identity.users SET status='suspended' WHERE id=$1")
        .bind(a.user_id)
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("DELETE FROM factory_identity.sessions WHERE user_id=$1")
        .bind(a.user_id)
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("UPDATE factory_identity.environments SET desired_state='stopped', generation=2 WHERE id=$1").bind(env).execute(&sql).await.unwrap();
    let obs = EnvironmentObservation {
        state: ObservedState::Stopped,
        error: None,
        volume_confirmed: true,
    };
    assert!(envs.record_observation(env, 2, obs, 1).await.unwrap());
    let count = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM factory_identity.model_probe_leases WHERE id=$1",
        )
        .bind(lease)
        .fetch_one(&sql)
        .await
        .unwrap()
    };
    assert_eq!(count().await, 1);
    // The trusted launch owner may record its conclusive return after revocation;
    // it cannot admit a user operation or authenticate the suspended owner.
    store
        .settle_model_probe_launch(lease, env, 1)
        .await
        .unwrap();
    assert_eq!(count().await, 1);
    assert!(envs.record_observation(env, 2, obs, 1).await.unwrap());
    assert_eq!(count().await, 0);
    assert!(matches!(
        store.has_open_runs(a).await,
        Err(ConversationError::Unauthorized)
    ));
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn project_conversation_requires_ready_owned_project_and_persists_association() {
    let (db, sql) = fixture().await;
    let store = db.conversation_store();
    let (one, env1) = actor(&sql, 220).await;
    let (two, env2) = actor(&sql, 221).await;
    let project = Uuid::new_v4();
    let operation = Uuid::new_v4();
    sqlx::query("INSERT INTO factory_identity.projects(id,user_id,repository_id,full_name,cloned_full_name,directory,state,operation_id,environment_id,environment_generation) VALUES($1,$2,1,'owner/repo','owner/repo',$3,'importing',$4,$5,1)")
        .bind(project).bind(one.user_id).bind("repo").bind(operation).bind(env1)
        .execute(&sql).await.unwrap();
    assert!(matches!(
        store.create_in_project(one, env1, 1, Some(project)).await,
        Err(ConversationError::NotFound)
    ));
    sqlx::query("UPDATE factory_identity.projects SET state='ready' WHERE id=$1")
        .bind(project)
        .execute(&sql)
        .await
        .unwrap();
    assert!(matches!(
        store.create_in_project(two, env2, 1, Some(project)).await,
        Err(ConversationError::NotFound)
    ));
    let conversation = store
        .create_in_project(one, env1, 1, Some(project))
        .await
        .unwrap();
    assert_eq!(conversation.project_id, Some(project));
    assert_eq!(
        store
            .history(one, env1, 1, conversation.id)
            .await
            .unwrap()
            .conversation
            .project_id,
        Some(project)
    );
    assert!(matches!(
        store.history(two, env2, 1, conversation.id).await,
        Err(ConversationError::NotFound)
    ));
}
