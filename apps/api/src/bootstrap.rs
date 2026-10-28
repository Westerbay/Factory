use crate::{app, config::RuntimeConfig};

pub async fn serve(config: RuntimeConfig) -> Result<(), &'static str> {
    let router = app::router(&config)?;
    let listener = tokio::net::TcpListener::bind(config.bind())
        .await
        .map_err(|_| "API_BIND_FAILED")?;
    tracing::info!(event = "api.listening", origin = config.api_origin());
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|_| "API_SERVE_FAILED")
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
