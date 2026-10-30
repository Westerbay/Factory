//! Explicit local operator commands. Never invoked by the API or build process.
use factory_adapters::identity::lookup_github_account;
use factory_adapters::{
    egress::EgressMetadata,
    environments::{DockerEnvironmentUpgradeRuntime, InstallationGuard, PreparationMetadata},
};
use factory_api::config::RuntimeConfig;
use factory_application::environments::{EnvironmentAvailability, EnvironmentUpgradeService};
use factory_core::environments::{EnvironmentError, EnvironmentStart};
use factory_core::identity::IdentityError;
use factory_database::{identity::IdentityDatabase, local_target, migrations::Readiness};
use std::{
    io::{self, BufRead, Write},
    process::ExitCode,
    sync::Arc,
};
use uuid::Uuid;

const USAGE: &str = "Usage: pnpm factory admin setup --provider github --username <GitHub username>\n       pnpm factory environment upgrade --user-id <Factory user UUID>";

enum Command<'a> {
    Help,
    Admin(&'a str),
    Upgrade(Uuid),
}

fn command(args: &[String]) -> Result<Command<'_>, &'static str> {
    if args.first().map(String::as_str) == Some("environment") {
        if args.len() != 4 || args[1] != "upgrade" || args[2] != "--user-id" {
            return Err("INVALID_ARGUMENTS");
        }
        let user = Uuid::parse_str(&args[3]).map_err(|_| "INVALID_USER_ID")?;
        if user.is_nil() || user.to_string() != args[3] {
            return Err("INVALID_USER_ID");
        }
        return Ok(Command::Upgrade(user));
    }
    Ok(match username(args)? {
        Some(name) => Command::Admin(name),
        None => Command::Help,
    })
}

fn username(args: &[String]) -> Result<Option<&str>, &'static str> {
    if args == ["--help"] || args == ["-h"] {
        return Ok(None);
    }
    if args.len() != 6 || args[0] != "admin" || args[1] != "setup" {
        return Err("INVALID_ARGUMENTS");
    }
    let mut provider = None;
    let mut username = None;
    for pair in args[2..].chunks_exact(2) {
        match pair[0].as_str() {
            "--provider" if provider.is_none() => provider = Some(pair[1].as_str()),
            "--username" if username.is_none() => username = Some(pair[1].as_str()),
            _ => return Err("INVALID_ARGUMENTS"),
        }
    }
    if provider != Some("github") {
        return Err("UNSUPPORTED_PROVIDER");
    }
    username
        .filter(|value| !value.is_empty())
        .map(Some)
        .ok_or("INVALID_ARGUMENTS")
}

fn confirm(input: impl BufRead, mut output: impl Write, handle: &str) -> io::Result<bool> {
    writeln!(
        output,
        "Type {handle} to confirm, or press Enter to cancel:"
    )?;
    output.flush()?;
    let mut answer = String::new();
    // Bound redirected input as well as interactive input. EOF never confirms.
    input.take(256).read_line(&mut answer)?;
    Ok(answer.ends_with('\n') && answer.trim_end_matches(['\r', '\n']) == handle)
}

async fn configure(database: &IdentityDatabase, username: &str) -> Result<(), &'static str> {
    ready(database).await?;
    if database
        .has_administrator()
        .await
        .map_err(|_| "IDENTITY_DATABASE_UNAVAILABLE")?
    {
        return Err("ADMIN_ALREADY_CONFIGURED");
    }
    let account = lookup_github_account(username)
        .await
        .map_err(|error| match error {
            IdentityError::NotFound => "GITHUB_ACCOUNT_NOT_FOUND",
            IdentityError::InvalidInput | IdentityError::InvalidLogin => "GITHUB_ACCOUNT_INVALID",
            _ => "GITHUB_LOOKUP_UNAVAILABLE",
        })?;
    println!(
        "Initial administrator: {} (GitHub ID {})",
        account.handle, account.subject
    );
    println!("Profile: https://github.com/{}", account.handle);
    if !confirm(io::stdin().lock(), io::stdout(), &account.handle)
        .map_err(|_| "CONFIRMATION_FAILED")?
    {
        println!("Cancelled. No administrator was configured.");
        return Ok(());
    }
    database
        .setup_administrator(&account)
        .await
        .map_err(|error| match error {
            IdentityError::ProtectedAdministrator => "ADMIN_ALREADY_CONFIGURED",
            _ => "ADMIN_SETUP_FAILED",
        })?;
    println!(
        "Administrator configured. Sign in with GitHub in Factory; no session was created here."
    );
    Ok(())
}

async fn ready(database: &IdentityDatabase) -> Result<(), &'static str> {
    match database.ready().await {
        Ok(Readiness::Ready) => {}
        Ok(Readiness::Missing | Readiness::Pending) => return Err("IDENTITY_MIGRATION_REQUIRED"),
        Ok(Readiness::Invalid) => return Err("IDENTITY_SCHEMA_INVALID"),
        Err(_) => return Err("IDENTITY_DATABASE_UNAVAILABLE"),
    }
    Ok(())
}

fn upgrade_error(error: EnvironmentError) -> &'static str {
    error.code()
}

async fn upgrade(
    database: &IdentityDatabase,
    config: &RuntimeConfig,
    user_id: Uuid,
) -> Result<(), &'static str> {
    ready(database).await?;
    let directory = config.workspace().join(".factory/environments");
    let metadata = PreparationMetadata::load(&directory).map_err(upgrade_error)?;
    // Excludes the API for the entire preview, confirmation and recoverable replacement.
    let _installation =
        InstallationGuard::acquire(&directory).map_err(|_| "STOP_API_BEFORE_UPGRADE")?;
    let supervisor = database
        .environment_supervisor(metadata.installation_id())
        .await
        .map_err(|_| "ENVIRONMENT_SUPERVISOR_UNAVAILABLE")?;
    let target = EnvironmentStart {
        installation_id: metadata.installation_id(),
        image_id: metadata.image_id().to_owned(),
        policy_version: metadata.policy_version().to_owned(),
    };
    let availability = EnvironmentAvailability::default();
    let permitted = availability.clone();
    let runtime = DockerEnvironmentUpgradeRuntime::connect(
        metadata,
        EgressMetadata::load(&directory).map_err(upgrade_error)?,
        Arc::new(move || !permitted.is_failed()),
    )
    .map_err(upgrade_error)?;
    let mut service = EnvironmentUpgradeService::new(
        Arc::new(database.environment_store()),
        Arc::new(runtime),
        Box::new(supervisor),
        availability,
        target,
    )
    .map_err(upgrade_error)?;
    let preview = service.preview(user_id).await.map_err(upgrade_error)?;
    let env = &preview.snapshot.environment;
    println!(
        "User: {user_id}\nEnvironment: {}\nHome volume (preserved): {}",
        env.id, env.volume_name
    );
    println!(
        "Old container: {}",
        preview
            .snapshot
            .pending
            .as_ref()
            .map(|intent| intent.old_container_id.as_str())
            .or(preview.old_container_id.as_deref())
            .unwrap_or("absent")
    );
    println!(
        "Current image: {} (policy {})\nTarget image: {} (policy {})",
        env.image_id, env.policy_version, preview.target.image_id, preview.target.policy_version
    );
    if preview.already_current {
        println!("Already current. No replacement is needed.");
        return Ok(());
    }
    if let Some(intent) = &preview.snapshot.pending {
        println!("Resuming operation: {}", intent.operation_id);
    }
    println!(
        "The stopped container will be removed; its home is preserved. Start recreates the container later."
    );
    if !confirm(io::stdin().lock(), io::stdout(), &user_id.to_string())
        .map_err(|_| "CONFIRMATION_FAILED")?
    {
        println!("Cancelled. No upgrade was performed.");
        return Ok(());
    }
    service.execute(&preview).await.map_err(upgrade_error)?;
    println!("Upgrade recorded. Restart the API, then start the environment when ready.");
    Ok(())
}

async fn run() -> Result<(), &'static str> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = command(&args)?;
    if matches!(command, Command::Help) {
        println!("{USAGE}");
        return Ok(());
    }
    let config = RuntimeConfig::from_environment()?;
    let target = local_target::from_workspace(config.workspace())
        .await
        .map_err(|_| "OWNED_DATABASE_REQUIRED")?;
    let database = IdentityDatabase::connect(&target)
        .await
        .map_err(|_| "IDENTITY_DATABASE_UNAVAILABLE")?;
    let result = match command {
        Command::Admin(username) => configure(&database, username).await,
        Command::Upgrade(user) => upgrade(&database, &config, user).await,
        Command::Help => unreachable!(),
    };
    database.close().await;
    result
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => {
            eprintln!("{code}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_requires_one_canonical_stable_user_id_and_exact_confirmation() {
        let user = Uuid::new_v4();
        let args = vec![
            "environment".into(),
            "upgrade".into(),
            "--user-id".into(),
            user.to_string(),
        ];
        assert!(matches!(command(&args), Ok(Command::Upgrade(id)) if id == user));
        for invalid in [
            "Westerbay",
            "user@example.invalid",
            "00000000-0000-0000-0000-000000000000",
        ] {
            let mut args = args.clone();
            args[3] = invalid.into();
            assert!(command(&args).is_err());
        }
        let mut yes = args.clone();
        yes.push("--yes".into());
        assert!(command(&yes).is_err());
        assert!(!confirm(format!("{user}").as_bytes(), Vec::new(), &user.to_string()).unwrap());
        assert!(!confirm(b"yes\n".as_slice(), Vec::new(), &user.to_string()).unwrap());
        assert!(
            confirm(
                format!("{user}\n").as_bytes(),
                Vec::new(),
                &user.to_string()
            )
            .unwrap()
        );
    }

    #[test]
    fn command_rejects_ambiguous_or_unsupported_arguments() {
        for args in [
            "admin setup --provider gitlab --username user",
            "admin setup --username user --username other",
            "admin setup --provider github",
            "admin setup --provider github --username user --yes",
        ] {
            let args: Vec<String> = args.split_whitespace().map(str::to_owned).collect();
            assert!(username(&args).is_err());
        }
        let args: Vec<String> = "admin setup --username Westerbay --provider github"
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        assert_eq!(username(&args).unwrap(), Some("Westerbay"));
    }

    #[test]
    fn only_explicit_confirmation_of_the_displayed_account_allows_setup() {
        for input in [
            "",
            "\n",
            "yes\n",
            "other\n",
            "Westerbay-extra\n",
            "Westerbay",
        ] {
            assert!(!confirm(input.as_bytes(), Vec::new(), "Westerbay").unwrap());
        }
        assert!(confirm("Westerbay\r\n".as_bytes(), Vec::new(), "Westerbay").unwrap());
    }
}
