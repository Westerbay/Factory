use std::{fs, process::Command};

#[test]
fn api_rejects_invalid_dotenv_safely_while_contract_export_ignores_it() {
    let directory = std::env::temp_dir().join(format!("factory-startup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join(".env");
    fs::write(
        &path,
        "FACTORY_GITHUB_CLIENT_SECRET=\"synthetic-hidden-value\n",
    )
    .unwrap();

    let api = Command::new(env!("CARGO_BIN_EXE_factory-api"))
        .current_dir(&directory)
        .output()
        .unwrap();
    let export = Command::new(env!("CARGO_BIN_EXE_export-openapi"))
        .current_dir(&directory)
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();

    assert!(!api.status.success());
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&api.stdout),
        String::from_utf8_lossy(&api.stderr)
    );
    assert!(output.contains("DOTENV_INVALID"));
    assert!(!output.contains("synthetic-hidden-value"));
    assert!(export.status.success());
    let document: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(document["info"]["title"], "Factory API");
    assert!(!String::from_utf8_lossy(&export.stdout).contains("synthetic-hidden-value"));
}
