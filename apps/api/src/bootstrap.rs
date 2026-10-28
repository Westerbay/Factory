use crate::{
    app::{self, IdentityRuntime},
    auth::AuthState,
    config::RuntimeConfig,
};
use factory_adapters::identity::GitHubProvider;
use factory_application::identity::IdentityService;
use factory_database::{identity::IdentityDatabase, local_target, migrations::Readiness};
use std::sync::Arc;

async fn identity(config: &RuntimeConfig) -> Result<IdentityRuntime, &'static str> {
    let settings = config.identity().ok_or("IDENTITY_CONFIGURATION_REQUIRED")?;
    let provider = GitHubProvider::new(
        settings.client_id.clone(),
        settings.client_secret.clone(),
        format!("{}/api/auth/github/callback", config.api_origin()),
    )
    .map_err(|_| "GITHUB_CONFIGURATION_INVALID")?;
    let target = local_target::from_workspace(config.workspace())
        .await
        .map_err(|_| "OWNED_DATABASE_REQUIRED")?;
    let database = Arc::new(
        IdentityDatabase::connect(&target)
            .await
            .map_err(|_| "IDENTITY_DATABASE_UNAVAILABLE")?,
    );
    let code = match database.ready().await {
        Ok(Readiness::Ready) => None,
        Ok(Readiness::Missing | Readiness::Pending) => Some("IDENTITY_MIGRATION_REQUIRED"),
        Ok(Readiness::Invalid) => Some("IDENTITY_SCHEMA_INVALID"),
        Err(_) => Some("IDENTITY_DATABASE_UNAVAILABLE"),
    };
    if let Some(code) = code {
        database.close().await;
        return Err(code);
    }
    let service = Arc::new(IdentityService::new(
        Arc::new(database.store()),
        Arc::new(provider),
    ));
    Ok(IdentityRuntime {
        auth: AuthState::new(service, config),
        database,
    })
}

pub async fn serve(config: RuntimeConfig) -> Result<(), &'static str> {
    let identity = match identity(&config).await {
        Ok(identity) => Some(identity),
        Err(code) => {
            tracing::warn!(
                event = "api.identity.unavailable",
                code,
                guidance = "Check explicit OAuth configuration and owned database; run identity migrations explicitly, then restart."
            );
            None
        }
    };
    let database = identity.as_ref().map(|identity| identity.database.clone());
    let router = app::router(&config, identity)?;
    let listener = tokio::net::TcpListener::bind(config.bind())
        .await
        .map_err(|_| "API_BIND_FAILED")?;
    tracing::info!(event = "api.listening", origin = config.api_origin());
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|_| "API_SERVE_FAILED");
    if let Some(database) = database {
        database.close().await;
    }
    result
}

// Adapted from WebApp Template (MIT); no database or mail lifecycle is imported.
async fn shutdown() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
            }
            Err(_) => tracing::error!(event = "api.shutdown_signal.failed"),
        }
    }
    #[cfg(not(unix))]
    if tokio::signal::ctrl_c().await.is_err() {
        tracing::error!(event = "api.shutdown_signal.failed");
    }
}
