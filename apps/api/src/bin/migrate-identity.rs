//! Explicit operator command. Never runs as part of API startup or compilation.
use factory_database::{identity::IdentityDatabase, local_target};
use std::{path::PathBuf, process::ExitCode};

async fn migrate() -> Result<(), &'static str> {
    if std::env::args_os().len() != 1 {
        return Err("NO_ARGUMENTS_SUPPORTED");
    }
    let root = match std::env::var_os("FACTORY_WORKSPACE_ROOT") {
        Some(root) => PathBuf::from(root),
        None => std::env::current_dir().map_err(|_| "WORKSPACE_UNAVAILABLE")?,
    };
    let target = local_target::from_workspace(&root)
        .await
        .map_err(|_| "OWNED_DATABASE_REQUIRED")?;
    let database = IdentityDatabase::connect(&target)
        .await
        .map_err(|_| "IDENTITY_DATABASE_UNAVAILABLE")?;
    let result = database
        .migrate()
        .await
        .map_err(|_| "IDENTITY_MIGRATION_FAILED");
    database.close().await;
    result
}

#[tokio::main]
async fn main() -> ExitCode {
    match migrate().await {
        Ok(()) => {
            println!("Factory identity migrations applied to the verified local database.");
            ExitCode::SUCCESS
        }
        Err(code) => {
            eprintln!("{code}: verify the owned local database and schema before retrying.");
            ExitCode::FAILURE
        }
    }
}
