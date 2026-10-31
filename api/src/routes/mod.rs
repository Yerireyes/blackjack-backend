pub mod achievements;
pub mod analyze;
pub mod auth;
pub mod casual;
pub mod cosmetics;
pub mod daily;
pub mod google;
pub mod play;
pub mod ranked;
pub mod sessions;
pub mod stats;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::state::AppState;

/// Estado del servicio y de la base de datos.
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "database": "ok" })),
        ),
        Err(e) => {
            tracing::warn!("health: base de datos no disponible: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "degraded", "database": "error" })),
            )
        }
    }
}
