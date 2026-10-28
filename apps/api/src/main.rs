use factory_api::{bootstrap, config::RuntimeConfig};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt().json().init();
    let config = match RuntimeConfig::from_values(|key| std::env::var(key).ok()) {
        Ok(config) => config,
        Err(code) => {
            tracing::error!(event = "api.configuration.failed", code);
            return std::process::ExitCode::FAILURE;
        }
    };
    match bootstrap::serve(config).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(code) => {
            tracing::error!(event = "api.runtime.failed", code);
            std::process::ExitCode::FAILURE
        }
    }
}
