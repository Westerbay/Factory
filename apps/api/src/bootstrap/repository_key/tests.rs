use super::*;

struct Fixture(PathBuf);
use std::path::PathBuf;

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("factory-key-test-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn generates_once_and_reuses_without_querying_credentials() {
    let fixture = Fixture::new();
    let first = load(&fixture.0, || async { Ok(false) }).await.unwrap();
    let second = load(&fixture.0, || async {
        panic!("existing key must not query")
    })
    .await
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.0.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn missing_key_with_existing_credentials_or_database_failure_is_not_replaced() {
    let fixture = Fixture::new();
    assert!(load(&fixture.0, || async { Ok(true) }).await.is_err());
    assert!(load(&fixture.0, || async { Err(()) }).await.is_err());
    assert!(!fixture.0.join(KEY_FILE).exists());
}

#[tokio::test]
async fn malformed_existing_key_is_never_replaced() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join(KEY_FILE), b"invalid fixture").unwrap();
    assert!(load(&fixture.0, || async { Ok(false) }).await.is_err());
    assert_eq!(
        fs::read(fixture.0.join(KEY_FILE)).unwrap(),
        b"invalid fixture"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn interrupted_publication_can_reuse_complete_key() {
    let fixture = Fixture::new();
    let key = load(&fixture.0, || async { Ok(false) }).await.unwrap();
    // A crash between linking the completed file and removing its temporary name.
    fs::hard_link(fixture.0.join(KEY_FILE), fixture.0.join(".interrupted.tmp")).unwrap();
    assert_eq!(load(&fixture.0, || async { Ok(true) }).await.unwrap(), key);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_and_insecure_permissions_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    fs::write(&target, [42; 32]).unwrap();
    symlink(&target, fixture.0.join(KEY_FILE)).unwrap();
    assert!(load(&fixture.0, || async { Ok(false) }).await.is_err());
    fs::remove_file(fixture.0.join(KEY_FILE)).unwrap();
    fs::write(fixture.0.join(KEY_FILE), [1; 32]).unwrap();
    fs::set_permissions(fixture.0.join(KEY_FILE), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load(&fixture.0, || async { Ok(false) }).await.is_err());
    assert_eq!(fs::read(&target).unwrap(), [42; 32]);
}
