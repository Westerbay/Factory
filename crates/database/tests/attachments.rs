use chrono::{Duration, Utc};
use factory_core::{
    attachments::*, conversations::*, environments::*, identity::CredentialHash, projects::*,
};
use factory_database::{
    identity::IdentityDatabase, local_target::from_fixture, projects::PgProjectStore,
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

fn file(size: u64) -> StoredFile {
    StoredFile {
        media_type: "image/png".into(),
        kind: MediaKind::Image,
        size,
        sha256: "a".repeat(64),
    }
}
async fn staged(
    store: &impl AttachmentStore,
    actor: EnvironmentActor,
    env: Uuid,
    conversation: Uuid,
    name: &str,
) -> Attachment {
    let id = Uuid::new_v4();
    let reserved = store
        .reserve(
            actor,
            env,
            1,
            NewAttachment {
                id,
                conversation_id: conversation,
                name: name.into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(reserved.state, AttachmentState::Uploading);
    store.mark_staged(actor, env, 1, id, file(7)).await.unwrap()
}
fn turn(conversation: Uuid, env: Uuid, operation: Uuid, ids: Vec<Uuid>) -> StartTurn {
    StartTurn {
        conversation_id: conversation,
        operation_id: operation,
        environment_id: env,
        generation: 1,
        model: "m".into(),
        input: "look".into(),
        attachment_ids: ids,
    }
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn staged_files_are_claimed_atomically_and_idempotently() {
    let (db, sql) = fixture().await;
    let files = db.attachment_store();
    let chats = db.conversation_store();
    let (owner, env) = actor(&sql, 1).await;
    let (other, other_env) = actor(&sql, 2).await;
    let conversation = chats.create(owner, env, 1).await.unwrap();
    let other_conversation = chats.create(other, other_env, 1).await.unwrap();
    let a = staged(&files, owner, env, conversation.id, "a.png").await;
    let b = staged(&files, owner, env, conversation.id, "b.png").await;
    let foreign = staged(&files, other, other_env, other_conversation.id, "c.png").await;

    // Ownership isolation: nobody else can see, claim or remove another member's file.
    assert_eq!(files.get(other, a.id).await, Err(AttachmentError::NotFound));
    assert_eq!(
        files.remove_staged(other, other_env, 1, a.id).await,
        Err(AttachmentError::NotFound)
    );
    assert_eq!(
        files.list(other, other_env, 1, conversation.id).await,
        Err(AttachmentError::NotFound)
    );
    assert_eq!(
        files
            .reserve(
                other,
                other_env,
                1,
                NewAttachment {
                    id: Uuid::new_v4(),
                    conversation_id: conversation.id,
                    name: "x".into()
                }
            )
            .await,
        Err(AttachmentError::NotFound)
    );
    // A failed claim leaves every file staged.
    let operation = Uuid::new_v4();
    assert_eq!(
        chats
            .begin_turn(
                owner,
                turn(conversation.id, env, operation, vec![a.id, foreign.id])
            )
            .await
            .map(|_| ()),
        Err(ConversationError::Conflict)
    );
    assert_eq!(
        files.get(owner, a.id).await.unwrap().state,
        AttachmentState::Staged
    );
    // Too many or duplicate IDs are rejected before any claim.
    assert_eq!(
        chats
            .begin_turn(
                owner,
                turn(conversation.id, env, Uuid::new_v4(), vec![a.id, a.id])
            )
            .await
            .map(|_| ()),
        Err(ConversationError::Invalid)
    );

    let request = turn(conversation.id, env, operation, vec![b.id, a.id]);
    let (run, fresh) = chats.begin_turn(owner, request.clone()).await.unwrap();
    assert!(fresh);
    let (again, fresh) = chats.begin_turn(owner, request.clone()).await.unwrap();
    assert_eq!((again.id, fresh), (run.id, false));
    assert_eq!(
        chats
            .find_turn(owner, request.clone())
            .await
            .unwrap()
            .unwrap()
            .id,
        run.id
    );
    // Reusing the operation with a different set conflicts.
    for ids in [vec![a.id], vec![], vec![a.id, b.id, Uuid::new_v4()]] {
        let changed = turn(conversation.id, env, operation, ids);
        assert_eq!(
            chats.find_turn(owner, changed.clone()).await.map(|_| ()),
            Err(ConversationError::Conflict)
        );
        assert_eq!(
            chats.begin_turn(owner, changed).await.map(|_| ()),
            Err(ConversationError::Conflict)
        );
    }
    // The same order-insensitive set is the same request.
    let reordered = turn(conversation.id, env, operation, vec![a.id, b.id]);
    assert_eq!(
        chats.find_turn(owner, reordered).await.unwrap().unwrap().id,
        run.id
    );

    // Attached files keep the order of the request and cannot be removed or reclaimed.
    let history = chats.history(owner, env, 1, conversation.id).await.unwrap();
    let user_message = history.messages[0].id;
    assert_eq!(
        history
            .attachments
            .iter()
            .map(|x| (x.id, x.message_id, x.state))
            .collect::<Vec<_>>(),
        vec![
            (b.id, Some(user_message), AttachmentState::Attached),
            (a.id, Some(user_message), AttachmentState::Attached)
        ]
    );
    assert_eq!(
        files.remove_staged(owner, env, 1, a.id).await,
        Err(AttachmentError::Conflict)
    );
    sqlx::query("UPDATE factory_identity.conversation_runs SET status='completed' WHERE id=$1")
        .bind(run.id)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(
        chats
            .begin_turn(
                owner,
                turn(conversation.id, env, Uuid::new_v4(), vec![a.id])
            )
            .await
            .map(|_| ()),
        Err(ConversationError::Conflict)
    );
    // Concurrent claims of one file: exactly one turn wins.
    let c = staged(&files, owner, env, conversation.id, "c.png").await;
    let first = chats.begin_turn(
        owner,
        turn(conversation.id, env, Uuid::new_v4(), vec![c.id]),
    );
    let second = chats.begin_turn(
        owner,
        turn(conversation.id, env, Uuid::new_v4(), vec![c.id]),
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.is_ok() as u8 + second.is_ok() as u8, 1);
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn staged_lifecycle_expiry_and_agent_captures_are_bounded_and_idempotent() {
    let (db, sql) = fixture().await;
    let files = db.attachment_store();
    let chats = db.conversation_store();
    let (owner, env) = actor(&sql, 3).await;
    let conversation = chats.create(owner, env, 1).await.unwrap();
    // The size limit and the stored classification are enforced.
    let id = Uuid::new_v4();
    files
        .reserve(
            owner,
            env,
            1,
            NewAttachment {
                id,
                conversation_id: conversation.id,
                name: "big.bin".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        files
            .mark_staged(owner, env, 1, id, file(MAX_ATTACHMENT_BYTES + 1))
            .await,
        Err(AttachmentError::TooLarge)
    );
    assert_eq!(
        files.mark_staged(owner, env, 2, id, file(1)).await,
        Err(AttachmentError::Conflict)
    );
    files
        .mark_staged(owner, env, 1, id, file(MAX_ATTACHMENT_BYTES))
        .await
        .unwrap();
    assert_eq!(
        files.mark_staged(owner, env, 1, id, file(1)).await,
        Err(AttachmentError::Conflict)
    );
    assert_eq!(
        files.remove_staged(owner, env, 1, id).await.unwrap().size,
        MAX_ATTACHMENT_BYTES
    );
    assert_eq!(files.get(owner, id).await, Err(AttachmentError::NotFound));

    // Expiry removes only this owner's old uploads.
    let old = staged(&files, owner, env, conversation.id, "old.png").await;
    let abandoned = Uuid::new_v4();
    files
        .reserve(
            owner,
            env,
            1,
            NewAttachment {
                id: abandoned,
                conversation_id: conversation.id,
                name: "half.png".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        files
            .expire_staged(owner, staged_cutoff(Utc::now()))
            .await
            .unwrap()
            .is_empty()
    );
    let mut expired = files
        .expire_staged(owner, Utc::now() + Duration::hours(1))
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect::<Vec<_>>();
    expired.sort();
    let mut expected = vec![old.id, abandoned];
    expected.sort();
    assert_eq!(expired, expected);

    // Agent files attach to the assistant message, captured or unavailable, once.
    let (run, _) = chats
        .begin_turn(owner, turn(conversation.id, env, Uuid::new_v4(), vec![]))
        .await
        .unwrap();
    let (message, _) = chats
        .append_assistant(owner, env, 1, run.id, "item-1", "done")
        .await
        .unwrap();
    let produced = vec![
        AgentAttachment {
            id: Uuid::new_v4(),
            name: "chart.png".into(),
            capture: AgentCapture::Captured(file(9)),
        },
        AgentAttachment {
            id: Uuid::new_v4(),
            name: "gone.txt".into(),
            capture: AgentCapture::Unavailable,
        },
    ];
    assert_eq!(
        files
            .record_agent_attachments(owner, env, 1, run.id, run.id, produced.clone())
            .await,
        Err(AttachmentError::NotFound)
    );
    let recorded = files
        .record_agent_attachments(owner, env, 1, run.id, message.id, produced.clone())
        .await
        .unwrap();
    assert_eq!(
        recorded.iter().map(|a| a.state).collect::<Vec<_>>(),
        vec![AttachmentState::Attached, AttachmentState::Unavailable]
    );
    assert_eq!(recorded[1].media_type, "application/octet-stream");
    assert_eq!(
        files
            .record_agent_attachments(owner, env, 1, run.id, message.id, produced.clone())
            .await
            .unwrap(),
        recorded
    );
    let mut different = produced.clone();
    different[0].id = Uuid::new_v4();
    assert_eq!(
        files
            .record_agent_attachments(owner, env, 1, run.id, message.id, different)
            .await,
        Err(AttachmentError::Conflict)
    );
    let too_many = (0..11)
        .map(|i| AgentAttachment {
            id: Uuid::new_v4(),
            name: format!("{i}.txt"),
            capture: AgentCapture::Unavailable,
        })
        .collect();
    assert_eq!(
        files
            .record_agent_attachments(owner, env, 1, run.id, message.id, too_many)
            .await,
        Err(AttachmentError::TooMany)
    );
    let history = chats.history(owner, env, 1, conversation.id).await.unwrap();
    assert_eq!(history.attachments, recorded);
    assert_eq!(
        files.list(owner, env, 1, conversation.id).await.unwrap(),
        recorded
    );
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn deleting_a_project_removes_its_attachment_rows_only() {
    let (db, sql) = fixture().await;
    let files = db.attachment_store();
    let chats = db.conversation_store();
    let projects = PgProjectStore::new(sql.clone());
    let (owner, env) = actor(&sql, 4).await;
    let (project, _) = projects
        .reserve(
            owner,
            env,
            1,
            ImportProject {
                repository_id: 900_001,
                full_name: "owner/attachments".into(),
                operation_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    projects
        .settle(owner, env, 1, project.id, project.operation_id, true)
        .await
        .unwrap();
    let inside = chats
        .create_in_project(owner, env, 1, Some(project.id))
        .await
        .unwrap();
    let outside = chats.create(owner, env, 1).await.unwrap();
    let kept = staged(&files, owner, env, outside.id, "keep.png").await;
    let a = staged(&files, owner, env, inside.id, "a.png").await;
    let (run, _) = chats
        .begin_turn(owner, turn(inside.id, env, Uuid::new_v4(), vec![a.id]))
        .await
        .unwrap();
    sqlx::query("UPDATE factory_identity.conversation_runs SET status='completed' WHERE id=$1")
        .bind(run.id)
        .execute(&sql)
        .await
        .unwrap();
    staged(&files, owner, env, inside.id, "pending.png").await;
    projects
        .begin_delete(owner, env, 1, project.id)
        .await
        .unwrap();
    projects
        .finish_delete(owner, env, 1, project.id)
        .await
        .unwrap();
    assert_eq!(files.get(owner, a.id).await, Err(AttachmentError::NotFound));
    assert_eq!(files.get(owner, kept.id).await.unwrap().id, kept.id);
    sql.close().await;
}
