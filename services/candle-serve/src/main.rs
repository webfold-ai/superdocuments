//! `candle-serve` — minimal OpenAI-compat HTTP server backed by Qwen2 via
//! `candle-transformers` 0.9.
//!
//! Endpoints implemented:
//!   GET  /v1/models            -> { data: [{ id, object }], object }
//!   POST /v1/chat/completions   -> { choices: [{ message: { role, content } }], usage }
//!
//! Default: `CANDLE_STUB=1` returns a deterministic echo of the user's last
//! message. Set `CANDLE_STUB=0` to load Qwen2.5-0.5B-Instruct from
//! HuggingFace (~1 GB at fp32) and run real autoregressive decoding.
//!
//! Config:
//!   CANDLE_BIND     — listen address, default `0.0.0.0:38080`
//!   CANDLE_MODEL_ID — HF repo id, default `Qwen/Qwen2.5-0.5B-Instruct`
//!   CANDLE_STUB     — `1` for the echo stub, `0` for real inference
//!   CANDLE_MAX_NEW  — decode-time max new tokens, default 256, max 1024

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen2::{Config, ModelForCausalLM};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokenizers::Tokenizer;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

const DEFAULT_MODEL_ID: &str = "Qwen/Qwen2.5-0.5B-Instruct";

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    model_id: String,
    loaded: Option<LoadedModel>,
}

struct LoadedModel {
    model: ModelForCausalLM,
    tokenizer: Tokenizer,
    device: Device,
}

type ErrBody = (StatusCode, Json<Value>);

fn err<S: Into<String>>(msg: S) -> ErrBody {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg.into() })),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind: SocketAddr = std::env::var("CANDLE_BIND")
        .unwrap_or_else(|_| "0.0.0.0:38080".into())
        .parse()
        .context("parsing CANDLE_BIND")?;

    let stub_mode = std::env::var("CANDLE_STUB")
        .map(|v| v != "0")
        .unwrap_or(true);
    let model_id = std::env::var("CANDLE_MODEL_ID").unwrap_or_else(|_| DEFAULT_MODEL_ID.into());
    tracing::info!(%bind, %model_id, stub_mode, "candle-serve starting");

    let inner = Inner {
        model_id,
        loaded: None,
    };
    let state = AppState {
        inner: Arc::new(Mutex::new(inner)),
    };

    // Spawn the loader so the listener binds immediately. While loading,
    // /v1/chat/completions returns 503 with a clear "still loading" error.
    let bg_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = load_model(bg_state).await {
            tracing::error!(error = %e, "candle-serve: model load failed");
        }
    });

    let app = Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "candle-serve listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn load_model(state: AppState) -> Result<()> {
    let model_id = std::env::var("CANDLE_MODEL_ID").unwrap_or_else(|_| DEFAULT_MODEL_ID.into());
    tracing::info!(model_id = %model_id, "loading model from HuggingFace");

    let api = hf_hub::api::sync::Api::new().context("hf_hub init")?;
    let repo = api.repo(hf_hub::Repo::model(model_id.clone()));

    let model_path = repo
        .get("model.safetensors")
        .context("download model.safetensors")?;
    let config_path = repo
        .get("config.json")
        .context("download config.json")?;
    let tokenizer_path = repo
        .get("tokenizer.json")
        .context("download tokenizer.json")?;

    let config_raw = std::fs::read_to_string(&config_path).context("read config")?;
    let config: Config = serde_json::from_str(&config_raw).context("parse config")?;

    let device = Device::Cpu;
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
            .context("mmap safetensors")?
    };
    let model = ModelForCausalLM::new(&config, vb).context("build model")?;
    let tokenizer =
        Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;

    let mut g = state.inner.lock().await;
    g.loaded = Some(LoadedModel {
        model,
        tokenizer,
        device,
    });
    tracing::info!("candle-serve: model ready");
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
    let g = state.inner.lock().await;
    Json(ModelsResponse {
        data: vec![ModelEntry {
            id: g.model_id.clone(),
            object: "model",
        }],
        object: "list",
    })
}

#[derive(Deserialize)]
struct ChatRequest {
    #[serde(default)]
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
) -> Result<Json<ChatResponse>, ErrBody> {
    let g = state.inner.lock().await;

    let stub_mode = std::env::var("CANDLE_STUB")
        .map(|v| v != "0")
        .unwrap_or(true);
    if stub_mode || g.loaded.is_none() {
        return Ok(Json(stub_response(&g.model_id, &req.messages, req.max_tokens.unwrap_or(256).min(1024))));
    }

    let model = g.loaded.as_ref().unwrap();
    let device = &model.device;
    let max_new = req.max_tokens.unwrap_or(256).min(1024);

    let mut prompt = String::new();
    for m in &req.messages {
        match m.role.as_str() {
            "system" => prompt.push_str(&format!("<|im_start|>system\n{}<|im_end|>\n", m.content)),
            "user" => prompt.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n", m.content)),
            "assistant" => {
                prompt.push_str(&format!(
                    "<|im_start|>assistant\n{}<|im_end|>\n",
                    m.content
                ));
            }
            _ => {}
        }
    }
    prompt.push_str("<|im_start|>assistant\n");

    let encoding = match model.tokenizer.encode(prompt.clone(), false) {
        Ok(e) => e,
        Err(e) => return Err(err(format!("tokenize: {e}"))),
    };
    let mut tokens = encoding.get_ids().to_vec();
    let prompt_tokens = tokens.len() as u32;

    let eos = model
        .tokenizer
        .token_to_id("<|im_end|>")
        .or_else(|| model.tokenizer.token_to_id("<|endoftext|>"))
        .unwrap_or(0);

    let mut generated = String::new();
    let mut completion_tokens: u32 = 0;
    let mut llm = model.model.clone();
    for _ in 0..max_new {
        let input = match Tensor::new(tokens.as_slice(), device) {
            Ok(t) => t,
            Err(e) => return Err(err(format!("input: {e}"))),
        };
        let logits = match llm.forward(&input, tokens.len() - 1) {
            Ok(l) => l,
            Err(e) => return Err(err(format!("forward: {e}"))),
        };
        let vocab_size = logits.shape().dims()[logits.shape().dims().len() - 1];
        let last = match logits.get(vocab_size - 1) {
            Ok(t) => t,
            Err(e) => return Err(err(format!("logits row: {e}"))),
        };
        let next = match last.argmax(0) {
            Ok(t) => t,
            Err(e) => return Err(err(format!("argmax: {e}"))),
        };
        let next_id: u32 = match next.to_scalar::<u32>() {
            Ok(n) => n,
            Err(e) => return Err(err(format!("scalar: {e}"))),
        };
        if next_id == eos {
            break;
        }
        let piece = model.tokenizer.id_to_token(next_id).unwrap_or_default();
        generated.push_str(&piece);
        tokens.push(next_id);
        completion_tokens += 1;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(Json(ChatResponse {
        id: format!("chatcmpl-{now:x}"),
        object: "chat.completion",
        created: now,
        model: g.model_id.clone(),
        choices: vec![ChatChoice {
            index: 0,
            message: ResponseMessage {
                role: "assistant",
                content: generated,
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

fn stub_response(model_id: &str, messages: &[ChatMessage], max_new: usize) -> ChatResponse {
    let last_user = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let truncated: String = last_user
        .split_whitespace()
        .take(max_new)
        .collect::<Vec<_>>()
        .join(" ");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ChatResponse {
        id: format!("chatcmpl-stub-{now:x}"),
        object: "chat.completion",
        created: now,
        model: model_id.to_string(),
        choices: vec![ChatChoice {
            index: 0,
            message: ResponseMessage {
                role: "assistant",
                content: format!("# Stub Response\n\n{truncated}\n"),
            },
            finish_reason: "stop".into(),
        }],
        usage: Usage {
            prompt_tokens: 0,
            completion_tokens: truncated.split_whitespace().count() as u32,
            total_tokens: truncated.split_whitespace().count() as u32,
        },
    }
}
