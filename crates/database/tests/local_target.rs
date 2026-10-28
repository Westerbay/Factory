use factory_database::local_target::{
    TargetError, TargetProof, check_proof, from_fixture, from_workspace,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    name: String,
    accepted: bool,
    proof: serde_json::Value,
}

#[test]
fn rust_ownership_proof_agrees_with_shared_typescript_acceptance_vectors() {
    let vectors: Vec<Vector> = serde_json::from_str(include_str!(
        "../../../scripts/database/target-fixtures.json"
    ))
    .unwrap();
    for vector in vectors {
        let accepted = serde_json::from_value::<TargetProof>(vector.proof)
            .ok()
            .is_some_and(|proof| check_proof(&proof).is_ok());
        assert_eq!(accepted, vector.accepted, "{}", vector.name);
    }
}

#[tokio::test]
async fn invalid_fixture_or_missing_workspace_never_reaches_docker() {
    let absent = std::path::Path::new("/factory-workspace-that-does-not-exist-36c650ed");
    assert!(from_workspace(absent).await.is_err());
    assert!(
        from_fixture(absent, "factory-local", "not-a-uuid", 54321)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn absent_non_file_oversized_or_invalid_markers_fail_before_inspection() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "factory-marker-test-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let state = root.join(".factory");
    std::fs::create_dir(&state).unwrap();
    let marker = state.join("database-owner");
    assert!(matches!(
        from_workspace(&root).await,
        Err(TargetError::MarkerUnavailable)
    ));
    std::fs::create_dir(&marker).unwrap();
    assert!(matches!(
        from_workspace(&root).await,
        Err(TargetError::MarkerUnavailable)
    ));
    std::fs::remove_dir(&marker).unwrap();
    for value in ["unknown".to_owned(), "x".repeat(129)] {
        std::fs::write(&marker, value).unwrap();
        assert!(matches!(
            from_workspace(&root).await,
            Err(TargetError::MarkerUnavailable)
        ));
    }
    std::fs::remove_file(&marker).unwrap();
    std::fs::remove_dir(&state).unwrap();
    std::fs::remove_dir(&root).unwrap();
}

/// Explicit integration hook; the caller must first create and own a unique fixture.
/// This hook performs inspection only and never provisions, connects, or removes Docker resources.
#[tokio::test]
#[ignore = "requires an explicitly owned running Docker fixture"]
async fn inspect_live_owned_fixture() {
    #[derive(Deserialize)]
    struct Fixture {
        project: String,
        owner: String,
        port: u16,
    }
    let path =
        std::env::var("FACTORY_TEST_TARGET_PATH").expect("explicit synthetic fixture target path");
    let metadata = std::fs::metadata(&path).expect("readable fixture metadata");
    assert!(
        metadata.is_file() && metadata.len() <= 4096,
        "bounded fixture metadata required"
    );
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(path).expect("readable fixture target"))
            .expect("fixture metadata shape");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = from_fixture(&root, &fixture.project, &fixture.owner, fixture.port)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(target.database(), "factory_test");
    assert_eq!(target.port(), fixture.port);
}
