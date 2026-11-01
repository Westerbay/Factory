use factory_core::identity::*;
use factory_database::{
    identity::{IdentityDatabase, PgIdentityStore},
    local_target::{VerifiedTarget, from_fixture},
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

async fn fixture() -> (VerifiedTarget, IdentityDatabase, PgPool) {
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
    let pool = PgPool::connect_with(options).await.unwrap();
    (target, db, pool)
}

fn hash(id: u32) -> CredentialHash {
    let mut hash = [0; 32];
    hash[..4].copy_from_slice(&id.to_be_bytes());
    CredentialHash(hash)
}

async fn admission(store: &PgIdentityStore, subject: &str, token: u32) -> LoginAdmission {
    store
        .create_login_attempt(NewLoginAttempt {
            state_hash: hash(token),
            binding_hash: hash(token + 100000),
            pkce_verifier: "a".repeat(43),
        })
        .await
        .unwrap();
    let attempt = store
        .take_login_attempt(hash(token), hash(token + 100000))
        .await
        .unwrap();
    LoginAdmission {
        identity: VerifiedIdentity {
            provider: "github".into(),
            issuer: "https://github.com".into(),
            subject: subject.into(),
            display_name: format!("User {subject}"),
            handle: format!("user{subject}"),
        },
        login_started_at: attempt.created_at,

        session: NewSession {
            token_hash: hash(token),
            csrf_token: "c".repeat(32),
            previous_token_hash: None,
        },
    }
}

fn active(outcome: AdmissionOutcome) -> ActiveSession {
    match outcome {
        AdmissionOutcome::Active(session) => session,
        _ => panic!("expected active admission"),
    }
}

async fn change(store: &PgIdentityStore, admin: u32, user: &User, status: AccessDecision) -> User {
    store
        .change_access(
            hash(admin),
            AccessRequest {
                target_id: user.id,
                status,
                expected_version: user.version,
            },
        )
        .await
        .unwrap()
}

async fn pending(store: &PgIdentityStore, admin: u32) -> Vec<User> {
    store
        .list_users(
            hash(admin),
            UserListQuery {
                status: AccessStatus::Pending,
                after: None,
                limit: 50,
            },
        )
        .await
        .unwrap()
        .items
}

async fn wait_for_blocked(sql: &PgPool, count: i64) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%FOR UPDATE OF u%'")
                .fetch_one(sql).await.unwrap();
            if blocked >= count { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("store operation reached the held user lock");
}

async fn lock_user(sql: &PgPool, id: Uuid) -> sqlx::Transaction<'_, sqlx::Postgres> {
    let mut tx = sql.begin().await.unwrap();
    sqlx::query("SELECT id FROM factory_identity.users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn operator_setup_is_atomic_preserves_sessions_on_failure_and_serializes_different_accounts()
{
    let (_, db, sql) = fixture().await;
    sqlx::query("TRUNCATE factory_identity.users, factory_identity.login_attempts CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    let store = db.store();
    let first = admission(&store, "1", 1).await.identity;
    let second = admission(&store, "2", 2).await.identity;
    assert!(!db.has_administrator().await.unwrap());

    // Failing promotion rolls back the new identity and pending user as well.
    sqlx::raw_sql("CREATE FUNCTION factory_identity.reject_admin_setup() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic rollback'; END $$; CREATE TRIGGER reject_admin_setup BEFORE UPDATE ON factory_identity.users FOR EACH ROW EXECUTE FUNCTION factory_identity.reject_admin_setup();").execute(&sql).await.unwrap();
    assert!(matches!(
        db.setup_administrator(&first).await,
        Err(IdentityError::Unavailable)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM factory_identity.users")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::raw_sql("DROP TRIGGER reject_admin_setup ON factory_identity.users; DROP FUNCTION factory_identity.reject_admin_setup();").execute(&sql).await.unwrap();

    let (one, two) = tokio::join!(
        db.setup_administrator(&first),
        db.setup_administrator(&second)
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let (selected, rejected) = if one.is_ok() {
        (&first, &second)
    } else {
        (&second, &first)
    };
    let admin = active(
        store
            .admit(admission(&store, &selected.subject, 10).await)
            .await
            .unwrap(),
    );
    assert_eq!(admin.user.role, Role::Admin);
    assert!(matches!(
        db.setup_administrator(rejected).await,
        Err(IdentityError::ProtectedAdministrator)
    ));
    assert!(matches!(
        db.setup_administrator(selected).await,
        Err(IdentityError::ProtectedAdministrator)
    ));
    assert_eq!(
        store.active_session(hash(10)).await.unwrap().unwrap().user,
        admin.user
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM factory_identity.users")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(count, 1);
    db.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn owned_identity_transactions_cover_admission_revocation_and_races() {
    let (target, db, sql) = fixture().await;
    sqlx::query("TRUNCATE factory_identity.users, factory_identity.login_attempts CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    let store = db.store();

    // State/binding consumption is single-use, atomic and rejects expiry/mismatch.
    store
        .create_login_attempt(NewLoginAttempt {
            state_hash: hash(1),
            binding_hash: hash(2),
            pkce_verifier: "v".repeat(43),
        })
        .await
        .unwrap();
    assert!(matches!(
        store.take_login_attempt(hash(1), hash(3)).await,
        Err(IdentityError::InvalidLogin)
    ));
    let (one, two) = tokio::join!(
        store.take_login_attempt(hash(1), hash(2)),
        store.take_login_attempt(hash(1), hash(2))
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    store
        .create_login_attempt(NewLoginAttempt {
            state_hash: hash(4),
            binding_hash: hash(5),
            pkce_verifier: "v".repeat(43),
        })
        .await
        .unwrap();
    sqlx::query("UPDATE factory_identity.login_attempts SET expires_at=clock_timestamp()-interval '1 second'").execute(&sql).await.unwrap();
    assert!(matches!(
        store.take_login_attempt(hash(4), hash(5)).await,
        Err(IdentityError::InvalidLogin)
    ));

    // Login cannot create an administrator, even before setup has run.
    let before_setup = admission(&store, "1", 101).await;
    let selected = before_setup.identity.clone();
    assert!(matches!(
        store
            .admit(admission(&store, "1", 100).await)
            .await
            .unwrap(),
        AdmissionOutcome::Pending
    ));
    let (one, two) = tokio::join!(
        db.setup_administrator(&selected),
        db.setup_administrator(&selected)
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    assert!(matches!(
        if one.is_err() { one } else { two },
        Err(IdentityError::ProtectedAdministrator)
    ));
    assert!(matches!(
        store.admit(before_setup).await.unwrap(),
        AdmissionOutcome::StaleLogin
    ));
    let admin_token = 102;
    let admin = active(
        store
            .admit(admission(&store, "1", admin_token).await)
            .await
            .unwrap(),
    )
    .user;
    let other = admission(&store, "9", 109).await;
    assert!(matches!(
        db.setup_administrator(&other.identity).await,
        Err(IdentityError::ProtectedAdministrator)
    ));
    assert!(matches!(
        store.admit(other).await.unwrap(),
        AdmissionOutcome::Pending
    ));
    assert!(matches!(
        store
            .admit(admission(&store, "2", 201).await)
            .await
            .unwrap(),
        AdmissionOutcome::Pending
    ));
    let mut member = pending(&store, admin_token)
        .await
        .into_iter()
        .find(|u| u.handle == "user2")
        .unwrap();
    let before_activation = admission(&store, "2", 202).await;
    member = change(&store, admin_token, &member, AccessDecision::Active).await;
    assert!(matches!(
        store.admit(before_activation).await.unwrap(),
        AdmissionOutcome::StaleLogin
    ));
    let first = active(
        store
            .admit(admission(&store, "2", 203).await)
            .await
            .unwrap(),
    );
    active(
        store
            .admit(admission(&store, "2", 204).await)
            .await
            .unwrap(),
    );
    assert_eq!(first.user.id, member.id);
    assert!(matches!(
        store
            .list_users(
                hash(203),
                UserListQuery {
                    status: AccessStatus::Pending,
                    after: None,
                    limit: 20
                }
            )
            .await,
        Err(IdentityError::Forbidden)
    ));
    assert!(matches!(
        store
            .change_access(
                hash(admin_token),
                AccessRequest {
                    target_id: member.id,
                    status: AccessDecision::Suspended,
                    expected_version: 1
                }
            )
            .await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        store
            .change_access(
                hash(admin_token),
                AccessRequest {
                    target_id: admin.id,
                    status: AccessDecision::Suspended,
                    expected_version: admin.version
                }
            )
            .await,
        Err(IdentityError::ProtectedAdministrator)
    ));
    let unchanged = change(&store, admin_token, &member, AccessDecision::Active).await;
    assert_eq!(unchanged, member);

    // Audit append failure rolls back both status and session revocation.
    sqlx::raw_sql("CREATE FUNCTION factory_identity.reject_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic rollback'; END $$; CREATE TRIGGER reject_event BEFORE INSERT ON factory_identity.access_events FOR EACH ROW EXECUTE FUNCTION factory_identity.reject_event();").execute(&sql).await.unwrap();
    assert!(matches!(
        store
            .change_access(
                hash(admin_token),
                AccessRequest {
                    target_id: member.id,
                    status: AccessDecision::Suspended,
                    expected_version: member.version
                }
            )
            .await,
        Err(IdentityError::Unavailable)
    ));
    assert!(store.active_session(hash(203)).await.unwrap().is_some());
    sqlx::raw_sql("DROP TRIGGER reject_event ON factory_identity.access_events; DROP FUNCTION factory_identity.reject_event();").execute(&sql).await.unwrap();
    member = change(&store, admin_token, &member, AccessDecision::Suspended).await;
    assert!(store.active_session(hash(203)).await.unwrap().is_none());
    assert!(store.active_session(hash(204)).await.unwrap().is_none());
    assert!(matches!(
        store
            .admit(admission(&store, "2", 205).await)
            .await
            .unwrap(),
        AdmissionOutcome::Suspended
    ));

    // Effective activation time must be sampled after waiting for the user lock.
    let held = lock_user(&sql, member.id).await;
    let task_store = store.clone();
    let request = AccessRequest {
        target_id: member.id,
        status: AccessDecision::Active,
        expected_version: member.version,
    };
    let activating =
        tokio::spawn(async move { task_store.change_access(hash(admin_token), request).await });
    wait_for_blocked(&sql, 1).await;
    let waiting_attempt = admission(&store, "2", 206).await;
    let started = waiting_attempt.login_started_at;
    held.commit().await.unwrap();
    member = activating.await.unwrap().unwrap();
    assert!(member.access_enabled_at.unwrap() > started);
    assert!(matches!(
        store.admit(waiting_attempt).await.unwrap(),
        AdmissionOutcome::StaleLogin
    ));
    active(
        store
            .admit(admission(&store, "2", 207).await)
            .await
            .unwrap(),
    );

    // Login and suspension compete for the same user row; neither ordering leaks a session.
    let login = admission(&store, "2", 208).await;
    let held = lock_user(&sql, member.id).await;
    let task_store = store.clone();
    let signing = tokio::spawn(async move { task_store.admit(login).await });
    let task_store = store.clone();
    let request = AccessRequest {
        target_id: member.id,
        status: AccessDecision::Suspended,
        expected_version: member.version,
    };
    let suspending =
        tokio::spawn(async move { task_store.change_access(hash(admin_token), request).await });
    wait_for_blocked(&sql, 2).await;
    held.commit().await.unwrap();
    signing.await.unwrap().unwrap();
    member = suspending.await.unwrap().unwrap();
    assert!(store.active_session(hash(207)).await.unwrap().is_none());
    assert!(store.active_session(hash(208)).await.unwrap().is_none());
    member = change(&store, admin_token, &member, AccessDecision::Active).await;
    active(
        store
            .admit(admission(&store, "2", 209).await)
            .await
            .unwrap(),
    );

    // Replacement/logout/restart/expiry depend on persisted state, never a cache.
    let mut replacement = admission(&store, "2", 210).await;
    replacement.session.previous_token_hash = Some(hash(209));
    active(store.admit(replacement).await.unwrap());
    assert!(store.active_session(hash(209)).await.unwrap().is_none());
    db.close().await;
    assert!(matches!(
        store.active_session(hash(210)).await,
        Err(IdentityError::Unavailable)
    ));
    let reopened = IdentityDatabase::connect(&target).await.unwrap();
    let store = reopened.store();
    assert!(store.active_session(hash(210)).await.unwrap().is_some());
    store.logout(hash(210)).await.unwrap();
    store.logout(hash(210)).await.unwrap();
    assert!(store.active_session(hash(210)).await.unwrap().is_none());
    active(
        store
            .admit(admission(&store, "2", 211).await)
            .await
            .unwrap(),
    );
    sqlx::query("UPDATE factory_identity.sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE token_hash=$1").bind(hash(211).0.as_slice()).execute(&sql).await.unwrap();
    assert!(store.active_session(hash(211)).await.unwrap().is_none());
    assert_eq!(member.status, AccessStatus::Active);
    reopened.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn repository_authorization_is_session_and_environment_bound() {
    use factory_core::environments::EnvironmentActor;
    let (target, db, sql) = fixture().await;
    assert!(!db.has_repository_credentials().await.unwrap());
    let user = Uuid::new_v4();
    let env = Uuid::new_v4();
    let subject = ((Uuid::new_v4().as_u128() % 9_000_000_000_000_000_000) + 1).to_string();
    let mut session = [0; 32];
    session[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    session[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Repository owner','member','active',clock_timestamp())").bind(user).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.external_identities(user_id,provider,issuer,subject,handle) VALUES($1,'github','https://github.com',$2,'fixture')").bind(user).bind(&subject).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')").bind(session.as_slice()).bind(user).bind("c".repeat(32)).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,volume_name,container_name,image_id,policy_version) VALUES($1,$2,$3,'running','running',$4,$5,'image','1')").bind(env).bind(user).bind(Uuid::new_v4()).bind(format!("repo-v-{env}")).bind(format!("repo-c-{env}")).execute(&sql).await.unwrap();
    let store = db.store();
    let actor = RepositoryActor {
        actor: EnvironmentActor {
            user_id: user,
            session_hash: CredentialHash(session),
        },
        environment_id: env,
        generation: 1,
    };
    let state = CredentialHash(
        *Uuid::new_v4()
            .as_bytes()
            .repeat(2)
            .first_chunk::<32>()
            .unwrap(),
    );
    let binding = CredentialHash(
        *Uuid::new_v4()
            .as_bytes()
            .repeat(2)
            .first_chunk::<32>()
            .unwrap(),
    );
    store
        .begin_repository_authorization(RepositoryAuthorizationAttempt {
            state_hash: state,
            binding_hash: binding,
            verifier: "v".repeat(43),
            actor,
            authorization_epoch: 0,
        })
        .await
        .unwrap();
    assert!(matches!(
        store
            .take_repository_authorization(state, binding, CredentialHash([9; 32]))
            .await,
        Err(RepositoryAccessError::InvalidLogin)
    ));
    let consumed = store
        .take_repository_authorization(state, binding, actor.actor.session_hash)
        .await
        .unwrap();
    assert_eq!(consumed.actor.actor.user_id, user);
    assert!(matches!(
        store
            .take_repository_authorization(state, binding, actor.actor.session_hash)
            .await,
        Err(RepositoryAccessError::InvalidLogin)
    ));
    let concurrent_state = CredentialHash(
        *Uuid::new_v4()
            .as_bytes()
            .repeat(2)
            .first_chunk::<32>()
            .unwrap(),
    );
    let concurrent_binding = CredentialHash(
        *Uuid::new_v4()
            .as_bytes()
            .repeat(2)
            .first_chunk::<32>()
            .unwrap(),
    );
    store
        .begin_repository_authorization(RepositoryAuthorizationAttempt {
            state_hash: concurrent_state,
            binding_hash: concurrent_binding,
            verifier: "v".repeat(43),
            actor,
            authorization_epoch: 0,
        })
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        store.take_repository_authorization(
            concurrent_state,
            concurrent_binding,
            actor.actor.session_hash
        ),
        store.take_repository_authorization(
            concurrent_state,
            concurrent_binding,
            actor.actor.session_hash
        )
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let mut identity = VerifiedIdentity {
        provider: "github".into(),
        issuer: "https://github.com".into(),
        subject: subject.clone(),
        display_name: "Fixture".into(),
        handle: "fixture".into(),
    };
    store
        .verify_repository_identity(actor, &identity, 1)
        .await
        .unwrap();
    identity.subject = if subject == "1" { "2" } else { "1" }.into();
    assert!(matches!(
        store.verify_repository_identity(actor, &identity, 1).await,
        Err(RepositoryAccessError::IdentityMismatch)
    ));
    identity.subject = subject.clone();
    store
        .save_repository_credential(
            actor,
            &identity,
            RepositoryCredential {
                subject: subject.clone(),
                authorization_epoch: 1,
                login: "fixture".into(),
                email: "fixture@example.test".into(),
                encrypted: EncryptedRepositoryToken {
                    key_id: "fixture".into(),
                    nonce: vec![1; 12],
                    ciphertext: vec![2; 16],
                },
            },
        )
        .await
        .unwrap();
    assert!(db.has_repository_credentials().await.unwrap());
    assert_eq!(
        store
            .repository_credential(actor)
            .await
            .unwrap()
            .unwrap()
            .email,
        "fixture@example.test"
    );
    // A consumed callback and a pending callback are both fenced by disconnect.
    let delayed_state = CredentialHash([31; 32]);
    let delayed_binding = CredentialHash([32; 32]);
    store
        .begin_repository_authorization(RepositoryAuthorizationAttempt {
            state_hash: delayed_state,
            binding_hash: delayed_binding,
            verifier: "v".repeat(43),
            actor,
            authorization_epoch: 0,
        })
        .await
        .unwrap();
    let delayed = store
        .take_repository_authorization(delayed_state, delayed_binding, actor.actor.session_hash)
        .await
        .unwrap();
    assert_eq!(delayed.authorization_epoch, 2);
    let pending_state = CredentialHash([33; 32]);
    let pending_binding = CredentialHash([34; 32]);
    store
        .begin_repository_authorization(RepositoryAuthorizationAttempt {
            state_hash: pending_state,
            binding_hash: pending_binding,
            verifier: "v".repeat(43),
            actor,
            authorization_epoch: 0,
        })
        .await
        .unwrap();
    store.delete_repository_credential(actor).await.unwrap();
    assert!(!db.has_repository_credentials().await.unwrap());
    assert!(matches!(
        store
            .take_repository_authorization(pending_state, pending_binding, actor.actor.session_hash)
            .await,
        Err(RepositoryAccessError::InvalidLogin)
    ));
    assert!(matches!(
        store
            .validate_repository_epoch(actor, delayed.authorization_epoch)
            .await,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
    let stale = RepositoryCredential {
        subject: subject.clone(),
        login: "fixture".into(),
        email: "fixture@example.test".into(),
        authorization_epoch: delayed.authorization_epoch,
        encrypted: EncryptedRepositoryToken {
            key_id: "fixture".into(),
            nonce: vec![1; 12],
            ciphertext: vec![2; 16],
        },
    };
    assert!(matches!(
        store
            .save_repository_credential(actor, &identity, stale)
            .await,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
    assert!(store.repository_credential(actor).await.unwrap().is_none());
    db.close().await;
    let db = IdentityDatabase::connect(&target).await.unwrap();
    let store = db.store();
    assert!(matches!(
        store
            .validate_repository_epoch(actor, delayed.authorization_epoch)
            .await,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));

    // A supervisor change holding the environment row must serialize with use.
    let mut env_change = sql.begin().await.unwrap();
    sqlx::query("UPDATE factory_identity.environments SET observed_state='stopped' WHERE id=$1")
        .bind(env)
        .execute(&mut *env_change)
        .await
        .unwrap();
    let waiter = {
        let store = store.clone();
        tokio::spawn(async move { store.validate_repository_epoch(actor, 3).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%factory_identity.environments%FOR UPDATE%'").fetch_one(&sql).await.unwrap();
            if blocked > 0 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("repository operation waited for the environment row");
    env_change.commit().await.unwrap();
    assert!(matches!(
        waiter.await.unwrap(),
        Err(RepositoryAccessError::Conflict)
    ));
    sqlx::query("UPDATE factory_identity.environments SET observed_state='running' WHERE id=$1")
        .bind(env)
        .execute(&sql)
        .await
        .unwrap();

    // A logout that has removed the session but not yet committed must block the
    // credential mutation; once logout commits, it must fail rather than save.
    let mut logout = sql.begin().await.unwrap();
    sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
        .bind(session.as_slice())
        .execute(&mut *logout)
        .await
        .unwrap();
    let credential = RepositoryCredential {
        subject: subject.clone(),
        login: "fixture".into(),
        email: "fixture@example.test".into(),
        authorization_epoch: 3,
        encrypted: EncryptedRepositoryToken {
            key_id: "fixture".into(),
            nonce: vec![1; 12],
            ciphertext: vec![2; 16],
        },
    };
    let writer = {
        let store = store.clone();
        let identity = identity.clone();
        tokio::spawn(async move {
            store
                .save_repository_credential(actor, &identity, credential)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%factory_identity.sessions%FOR UPDATE%'").fetch_one(&sql).await.unwrap();
            if blocked > 0 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("repository mutation waited for the session row");
    logout.commit().await.unwrap();
    assert!(matches!(
        writer.await.unwrap(),
        Err(RepositoryAccessError::Unauthorized)
    ));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM factory_identity.github_credentials WHERE user_id=$1",
    )
    .bind(user)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(count, 0);
    sql.close().await;
    db.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn login_repository_credential_requires_live_matching_session_and_epoch() {
    use factory_core::environments::EnvironmentActor;
    let (_, db, sql) = fixture().await;
    let user = Uuid::new_v4();
    let hash = CredentialHash([67; 32]);
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Fixture','member','active',clock_timestamp())").bind(user).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.external_identities(user_id,provider,issuer,subject,handle) VALUES($1,'github','https://github.com','123456','fixture')").bind(user).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.sessions(token_hash,user_id,csrf_token,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '1 day')").bind(hash.0.as_slice()).bind(user).bind("c".repeat(32)).execute(&sql).await.unwrap();
    let actor = EnvironmentActor {
        user_id: user,
        session_hash: hash,
    };
    let store = db.store();
    let identity = VerifiedIdentity {
        provider: "github".into(),
        issuer: "https://github.com".into(),
        subject: "123456".into(),
        display_name: "Fixture".into(),
        handle: "fixture".into(),
    };
    let credential = |epoch| RepositoryCredential {
        subject: "123456".into(),
        login: "fixture".into(),
        email: "fixture@example.test".into(),
        authorization_epoch: epoch,
        encrypted: EncryptedRepositoryToken {
            key_id: "fixture".into(),
            nonce: vec![1; 12],
            ciphertext: vec![2; 16],
        },
    };
    assert_eq!(store.login_credential_epoch(actor).await.unwrap(), 0);
    let mut wrong = identity.clone();
    wrong.subject = "654321".into();
    assert!(
        store
            .save_login_credential(actor, &wrong, credential(0))
            .await
            .is_err()
    );
    store
        .save_login_credential(actor, &identity, credential(0))
        .await
        .unwrap();
    assert_eq!(store.login_credential_epoch(actor).await.unwrap(), 1);
    assert!(matches!(
        store
            .save_login_credential(actor, &identity, credential(0))
            .await,
        Err(RepositoryAccessError::ProviderUnauthorized)
    ));
    sqlx::query("DELETE FROM factory_identity.sessions WHERE token_hash=$1")
        .bind(hash.0.as_slice())
        .execute(&sql)
        .await
        .unwrap();
    assert!(matches!(
        store
            .save_login_credential(actor, &identity, credential(1))
            .await,
        Err(RepositoryAccessError::Unauthorized)
    ));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM factory_identity.github_credentials WHERE user_id=$1",
    )
    .bind(user)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(count, 1);
    sql.close().await;
    db.close().await;
}
