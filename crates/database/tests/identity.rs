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
        bootstrap: BootstrapAdmin::github("1").unwrap(),
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
async fn owned_identity_transactions_cover_admission_revocation_and_races() {
    let (target, db, sql) = fixture().await;
    db.migrate().await.unwrap();
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

    // Bootstrap is stable under concurrent callbacks and later configuration changes.
    let a = admission(&store, "1", 101).await;
    let b = admission(&store, "1", 102).await;
    let (a, b) = tokio::join!(store.admit(a), store.admit(b));
    let (admin_token, admin) = match (a.unwrap(), b.unwrap()) {
        (AdmissionOutcome::Active(a), _) => (101, a.user),
        (_, AdmissionOutcome::Active(b)) => (102, b.user),
        _ => panic!("one bootstrap succeeds"),
    };
    let admins: i64 =
        sqlx::query_scalar("SELECT count(*) FROM factory_identity.users WHERE role='admin'")
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(admins, 1);
    let mut changed_config = admission(&store, "9", 109).await;
    changed_config.bootstrap = BootstrapAdmin::github("9").unwrap();
    assert!(matches!(
        store.admit(changed_config).await.unwrap(),
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
