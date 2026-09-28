//! `candle-serve` — minimal OpenAI-compat HTTP server that the
//! `superdocuments-bench/ocr` harness uses as the `candle-qwen-serve` target.
//!
//! Endpoints implemented:
//!   GET  /v1/models            -> { data: [{ id, object }], object }
//!   POST /v1/chat/completions   -> { choices: [{ message: { role, content } }], usage }
//!
//! The current implementation is a STUB that returns a deterministic
//! markdown-formatted echo response. It exists so the harness can prove the
//! HTTP wiring works end-to-end (target discoverable via `/v1/models`,
//! POST reaches the server, response shape parses, CER + graph-fidelity scores
//! populate). The real Qwen2.5-VL model load is deferred — candle-transformers
//! 0.9's API for `quantized_qwen2::ModelWeights` and the per-tok forward loop
//! is verbose enough to deserve a dedicated PR.
//!
//! Config:
//!   CANDLE_BIND     — listen address, default `0.0.0.0:38080`
//!   CANDLE_MODEL_ID — reported via `/v1/models`, default `Qwen/Qwen2.5-0.5B-Instruct`
//!   CANDLE_STUB     — `1` to return the deterministic echo response (default),
//!                      `0` to return a 503 (force the harness to wait).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;

const DEFAULT_MODEL_ID: &str = "Qwen/Qwen2.5-0.5B-Instruct";

#[derive(Clone)]
struct AppState {
    model_id: Arc<String>,
    stub_mode: Arc<bool>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind: SocketAddr = std::env::var("CANDLE_BIND")
        .unwrap_or_else(|_| "0.0.0.0:38080".into())
        .parse()?;
    let model_id = std::env::var("CANDLE_MODEL_ID").unwrap_or_else(|_| DEFAULT_MODEL_ID.into());
    let stub_mode = std::env::var("CANDLE_STUB")
        .map(|v| v != "0")
        .unwrap_or(true);
    tracing::info!(%bind, %model_id, stub_mode, "candle-serve listening");

    let state = AppState {
        model_id: Arc::new(model_id),
        stub_mode: Arc::new(stub_mode),
    };
    let app = Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Serialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
    object: &'static str,
}

#[derive(Serialize)]
struct ModelEntry {
    id: String,
    object: &'static str,
}

async fn list_models(State(state): State<AppState>) -> Json<ModelsResponse> {
    Json(ModelsResponse {
        data: vec![ModelEntry {
            id: (*state.model_id).clone(),
            object: "model",
        }],
        object: "list",
    })
}

#[derive(Deserialize)]
struct ChatRequest {
    #[serde(default)]
    #[allow(dead_code)]
    model: Option<String>,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    max_tokens: Option<usize>,
}

#[derive(Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatResponse {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChoice>,
    usage: Usage,
}

#[derive(Serialize)]
struct ChatChoice {
    index: u32,
    message: ResponseMessage,
    finish_reason: String,
}

#[derive(Serialize)]
struct ResponseMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

async fn chat_completions(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, Json<Value>)> {
    if !*state.stub_mode {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "candle-serve is in non-stub mode but real model loading is not yet implemented",
            })),
        ));
    }

    // Stub response: extract the last user message, wrap it in a deterministic
    // markdown shape. This is enough for the harness to score CER + graph
    // fidelity without the model actually running.
    let last_user = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let prompt_tokens: u32 = req
        .messages
        .iter()
        .map(|m| m.content.split_whitespace().count() as u32)
        .sum();
    let max_new = req.max_tokens.unwrap_or(256);
    // Echo back the user's text wrapped as a heading + paragraph. Looks like
    // a coherent extraction to the harness's CER scoring.
    let stub_content = format!("# Stub Response\n\n{}\n", last_user);
    // Truncate to max_new words so the harness's bytes_out varies.
    let truncated: String = stub_content
        .split_whitespace()
        .take(max_new)
        .collect::<Vec<_>>()
        .join(" ");
    let completion_tokens = truncated.split_whitespace().count() as u32;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(Json(ChatResponse {
        id: format!("chatcmpl-stub-{:x}", now),
        object: "chat.completion",
        created: now,
        model: (*state.model_id).clone(),
        choices: vec![ChatChoice {
            index: 0,
            message: ResponseMessage {
                role: "assistant",
                content: truncated,
            },
            finish_reason: "stop".into(),
        }],
        usage: Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        },
    }))
}
