use axum::Json;
use axum::response::IntoResponse;
use serde_json::json;

pub async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "trading-server",
        "timestamp": "2026-10-24T20:00:00Z"
    }))
}
