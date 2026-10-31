//! Explicit local operator commands. Never invoked by the API or build process.
use factory_adapters::identity::lookup_github_account;
use factory_api::config::RuntimeConfig;
use factory_core::identity::IdentityError;
use factory_database::{identity::IdentityDatabase, local_target, schema::Readiness};
use std::{
    io::{self, BufRead, Write},
    process::ExitCode,
};

const USAGE: &str =
    "Usage: pnpm factory admin setup --provider github --username <GitHub username>";

enum Command<'a> {
    Help,
    Admin(&'a str),
}

fn command(args: &[String]) -> Result<Command<'_>, &'static str> {
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
        Ok(Readiness::Missing) => return Err("IDENTITY_SCHEMA_REQUIRED"),
        Ok(Readiness::Invalid) => return Err("IDENTITY_SCHEMA_INVALID"),
        Err(_) => return Err("IDENTITY_DATABASE_UNAVAILABLE"),
    }
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
