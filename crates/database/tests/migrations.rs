use factory_database::{
    identity::IdentityDatabase, local_target::from_fixture, migrations::Readiness,
};
use serde::Deserialize;
use sqlx::{PgPool, Row, postgres::PgConnectOptions};

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
    (db, PgPool::connect_with(options).await.unwrap())
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn owned_migrations_are_isolated_locked_and_fail_closed() {
    let (db, sql) = fixture().await;
    // Destructive cases are confined to this uniquely-owned test schema, never public/Drizzle.
    sqlx::query("DROP SCHEMA IF EXISTS factory_identity CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Missing);
    let absent: bool = sqlx::query_scalar("SELECT to_regnamespace('factory_identity') IS NULL")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert!(absent, "readiness must not create its schema or ledger");
    let (one, two) = tokio::join!(db.migrate(), db.migrate());
    one.unwrap();
    two.unwrap();
    db.migrate().await.unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    let public_ledger: bool =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NULL")
            .fetch_one(&sql)
            .await
            .unwrap();
    assert!(public_ledger);
    let original =
        sqlx::query("SELECT checksum FROM factory_identity._sqlx_migrations WHERE version=1")
            .fetch_one(&sql)
            .await
            .unwrap()
            .get::<Vec<u8>, _>("checksum");
    for update in [
        "UPDATE factory_identity._sqlx_migrations SET checksum='bad'::bytea WHERE version=1",
        "UPDATE factory_identity._sqlx_migrations SET success=false WHERE version=1",
        "UPDATE factory_identity._sqlx_migrations SET version=999 WHERE version=1",
    ] {
        sqlx::query(update).execute(&sql).await.unwrap();
        assert_eq!(db.ready().await.unwrap(), Readiness::Invalid);
        assert!(db.migrate().await.is_err());
        sqlx::query(
            "UPDATE factory_identity._sqlx_migrations SET version=1,success=true,checksum=$1 WHERE version IN (1,999)",
        )
        .bind(&original)
        .execute(&sql)
        .await
        .unwrap();
    }
    sqlx::query("CREATE TABLE factory_identity.unrecognized(id int)")
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Invalid);
    assert!(db.migrate().await.is_err());
    sqlx::query("DROP TABLE factory_identity.unrecognized")
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("DROP TABLE factory_identity._sqlx_migrations")
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Invalid);
    assert!(db.migrate().await.is_err());
    sqlx::query("DROP SCHEMA factory_identity CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA factory_identity")
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Pending);
    db.migrate().await.unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    db.close().await;
    assert!(db.ready().await.is_err());
    assert!(db.migrate().await.is_err());
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn environment_upgrade_preserves_identity_and_rolls_back_partial_ddl() {
    let (db, sql) = fixture().await;
    sqlx::query("DROP SCHEMA IF EXISTS factory_identity CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA factory_identity")
        .execute(&sql)
        .await
        .unwrap();
    // Start with the real version-one migration and SQLx checksum ledger.
    let all = sqlx::migrate!("./migrations");
    let first = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            all.iter().filter(|m| m.version == 1).cloned().collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    let mut connection = sql.acquire().await.unwrap();
    sqlx::query("SET search_path TO factory_identity")
        .execute(&mut *connection)
        .await
        .unwrap();
    first.run(&mut *connection).await.unwrap();
    drop(connection);
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status) VALUES($1,'Preserved member','member','pending')")
        .bind(id).execute(&sql).await.unwrap();
    let checksum: Vec<u8> = sqlx::query_scalar(
        "SELECT checksum FROM factory_identity._sqlx_migrations WHERE version=1",
    )
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Pending);
    // Deliberately collide late in migration 2. SQLx must roll back its prior CREATE TABLE.
    sqlx::query("CREATE INDEX environments_reconcile ON factory_identity.users(display_name)")
        .execute(&sql)
        .await
        .unwrap();
    assert!(db.migrate().await.is_err());
    let absent: bool =
        sqlx::query_scalar("SELECT to_regclass('factory_identity.environments') IS NULL")
            .fetch_one(&sql)
            .await
            .unwrap();
    assert!(absent);
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM factory_identity._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&sql)
    .await
    .unwrap();
    assert_eq!(versions, vec![1]);
    sqlx::query("DROP INDEX factory_identity.environments_reconcile")
        .execute(&sql)
        .await
        .unwrap();
    db.migrate().await.unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    let name: String =
        sqlx::query_scalar("SELECT display_name FROM factory_identity.users WHERE id=$1")
            .bind(id)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(name, "Preserved member");
    let preserved: Vec<u8> = sqlx::query_scalar(
        "SELECT checksum FROM factory_identity._sqlx_migrations WHERE version=1",
    )
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(preserved, checksum);
    sqlx::query("DROP TABLE factory_identity.environments CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Invalid);
    assert!(db.migrate().await.is_err());
    // Restore this disposable schema for the next suite.
    sqlx::query("DROP SCHEMA factory_identity CASCADE")
        .execute(&sql)
        .await
        .unwrap();
    db.migrate().await.unwrap();
    db.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn persistent_upgrade_preserves_existing_environment_records() {
    let (db, sql) = fixture().await;
    sqlx::raw_sql(
        "DROP SCHEMA IF EXISTS factory_identity CASCADE; CREATE SCHEMA factory_identity;",
    )
    .execute(&sql)
    .await
    .unwrap();
    let all = sqlx::migrate!("./migrations");
    let legacy = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            all.iter().filter(|m| m.version <= 2).cloned().collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    let mut connection = sql.acquire().await.unwrap();
    sqlx::query("SET search_path TO factory_identity")
        .execute(&mut *connection)
        .await
        .unwrap();
    legacy.run(&mut *connection).await.unwrap();
    drop(connection);
    let user = uuid::Uuid::new_v4();
    let environment = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status,access_enabled_at) VALUES($1,'Preserved user','member','active',clock_timestamp())")
        .bind(user).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,volume_name,container_name,image_id,policy_version,capacity_reserved,lease_expires_at) VALUES($1,$2,$3,'running','running','preserved-home','preserved-container','sha256:preserved','1',true,clock_timestamp()-interval '1 day')")
        .bind(environment).bind(user).bind(uuid::Uuid::new_v4()).execute(&sql).await.unwrap();
    let before: String = sqlx::query_scalar(
        "SELECT row_to_json(e)::text FROM factory_identity.environments e WHERE id=$1",
    )
    .bind(environment)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Pending);
    db.migrate().await.unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    let after: String = sqlx::query_scalar(
        "SELECT row_to_json(e)::text FROM factory_identity.environments e WHERE id=$1",
    )
    .bind(environment)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "migration must preserve existing user storage identities and intent"
    );
    // Persistent running intent no longer requires either legacy field.
    sqlx::query("UPDATE factory_identity.environments SET capacity_reserved=false,lease_expires_at=NULL WHERE id=$1")
        .bind(environment).execute(&sql).await.unwrap();
    db.close().await;
    sql.close().await;
}

#[tokio::test]
#[ignore = "requires the explicitly owned PostgreSQL integration fixture"]
async fn replacement_intent_migration_preserves_version_three_rows() {
    let (db, sql) = fixture().await;
    sqlx::raw_sql(
        "DROP SCHEMA IF EXISTS factory_identity CASCADE; CREATE SCHEMA factory_identity;",
    )
    .execute(&sql)
    .await
    .unwrap();
    let all = sqlx::migrate!("./migrations");
    let legacy = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            all.iter().filter(|m| m.version <= 3).cloned().collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    let mut connection = sql.acquire().await.unwrap();
    sqlx::query("SET search_path TO factory_identity")
        .execute(&mut *connection)
        .await
        .unwrap();
    legacy.run(&mut *connection).await.unwrap();
    drop(connection);
    let user = uuid::Uuid::new_v4();
    let env = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO factory_identity.users(id,display_name,role,status) VALUES($1,'Migration fixture','member','pending')")
        .bind(user).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO factory_identity.environments(id,user_id,installation_id,desired_state,observed_state,provisioning_state,volume_name,container_name,image_id,policy_version) VALUES($1,$2,$3,'stopped','stopped','volume_ready','existing-home','existing-container','sha256:legacy','2')")
        .bind(env).bind(user).bind(uuid::Uuid::new_v4()).execute(&sql).await.unwrap();
    let before: String = sqlx::query_scalar(
        "SELECT row_to_json(e)::text FROM factory_identity.environments e WHERE id=$1",
    )
    .bind(env)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Pending);
    db.migrate().await.unwrap();
    assert_eq!(db.ready().await.unwrap(), Readiness::Ready);
    let after: String = sqlx::query_scalar(
        "SELECT row_to_json(e)::text FROM factory_identity.environments e WHERE id=$1",
    )
    .bind(env)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(before, after);
    let intents: i64 =
        sqlx::query_scalar("SELECT count(*) FROM factory_identity.environment_upgrades")
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(
        intents, 0,
        "migration must not schedule replacement of existing resources"
    );
    db.close().await;
    sql.close().await;
}
