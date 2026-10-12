//! POST /v1/chat — grounded multi-turn chat with the fixed tool set.
//! T0: request DTO + honest 501. T4 implements the bounded tool loop
//! (system prompt = preamble + category + kb.index_prompt()).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::http::AppState;

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub conversation_id: Option<String>,
    pub message: String,
    #[serde(default = "default_stream")]
    pub stream: bool,
}

fn default_stream() -> bool {
    true
}

pub async fn handler(
    State(_state): State<Arc<AppState>>,
    Json(_req): Json<ChatRequest>,
) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"schema": 1, "error": "chat lands in T4"})),
    )
        .into_response()
}
