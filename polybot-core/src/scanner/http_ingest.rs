use axum::{extract::State, http::StatusCode, routing::post, Router};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing;

use crate::config::AppConfig;
use crate::scanner::schema::validate_and_create_event_with_max_age;
use polybot_common::errors::PolybotError;
use polybot_common::types::ScannerEvent;

fn load_http_ingest_api_key() -> Option<String> {
    let api_key = std::env::var("POLYBOT_API_KEY").ok()?;
    let api_key = api_key.trim();
    if api_key.is_empty() || api_key == "default-api-key" {
        return None;
    }

    Some(api_key.to_string())
}

fn http_ingest_bind_addr(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], port))
}

#[derive(Clone)]
pub struct AppState {
    pub signal_sender: mpsc::Sender<ScannerEvent>,
    pub api_key: String,
    pub signal_max_age_secs: u64,
}

async fn ingest_signal(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<StatusCode, StatusCode> {
    // Validate API key
    if let Some(key) = headers.get("X-API-Key") {
        if key.to_str().unwrap_or("") != state.api_key {
            tracing::warn!("Invalid API key from HTTP ingestion");
            return Err(StatusCode::UNAUTHORIZED);
        }
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match validate_and_create_event_with_max_age(&body, state.signal_max_age_secs) {
        Ok(event) => {
            tracing::info!(
                signal_id = %event.signal.signal_id,
                "Received signal via HTTP"
            );
            state
                .signal_sender
                .send(event)
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            Ok(StatusCode::OK)
        }
        Err(e) => {
            tracing::error!(error = %e, "Invalid signal received via HTTP");
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

pub async fn start_http_server(
    config: &AppConfig,
    signal_sender: mpsc::Sender<ScannerEvent>,
) -> Result<(), PolybotError> {
    let Some(api_key) = load_http_ingest_api_key() else {
        tracing::warn!(
            "HTTP ingestion server disabled because POLYBOT_API_KEY is missing or still using the placeholder value"
        );
        return Ok(());
    };

    let state = Arc::new(AppState {
        signal_sender,
        api_key,
        signal_max_age_secs: config.scanner.signal_max_age_secs,
    });

    let app = Router::new()
        .route("/signals", post(ingest_signal))
        .with_state(state);

    let addr = http_ingest_bind_addr(config.scanner.http_port);
    tracing::info!("HTTP ingestion server starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| PolybotError::Scanner(format!("Failed to bind HTTP server: {}", e)))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| PolybotError::Scanner(format!("HTTP server error: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn missing_api_key_disables_http_ingest() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("POLYBOT_API_KEY");

        assert_eq!(load_http_ingest_api_key(), None);
    }

    #[test]
    fn placeholder_api_key_disables_http_ingest() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("POLYBOT_API_KEY", "default-api-key");

        assert_eq!(load_http_ingest_api_key(), None);

        std::env::remove_var("POLYBOT_API_KEY");
    }

    #[test]
    fn custom_api_key_enables_http_ingest() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("POLYBOT_API_KEY", "test-secret");

        assert_eq!(load_http_ingest_api_key().as_deref(), Some("test-secret"));

        std::env::remove_var("POLYBOT_API_KEY");
    }

    #[test]
    fn http_ingest_binds_to_loopback() {
        let addr = http_ingest_bind_addr(8081);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8081);
    }
}
