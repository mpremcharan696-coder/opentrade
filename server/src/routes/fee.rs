use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

pub async fn fee_config_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = &state.inner.fee_config;
    Json(serde_json::json!({
        "version": cfg.version,
        "fee_wallet": cfg.fee_wallet.to_string(),
        "tier": cfg.tier,
    }))
}
