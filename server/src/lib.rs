pub mod builder;
pub mod routes;
pub mod state;

use axum::Router;
use axum::routing::{get, post};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use routes::compare::compare_routes_handler;
use routes::fee::fee_config_handler;
use routes::health::health_handler;
use routes::intent::{create_buy_intent_handler, create_sell_intent_handler};
use routes::swap::prepare_swap_candidate_handler;
use state::AppState;

pub fn create_app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health_handler))
        .route("/api/v1/config/fees", get(fee_config_handler))
        .route("/api/v1/routes/compare", post(compare_routes_handler))
        .route("/api/v1/intent/buy", post(create_buy_intent_handler))
        .route("/api/v1/intent/sell", post(create_sell_intent_handler))
        .route("/api/v1/swap/prepare", post(prepare_swap_candidate_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
