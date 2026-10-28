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
            "UPDATE factory_identity._sqlx_migrations SET version=1,success=true,checksum=$1",
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
