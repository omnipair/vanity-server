#[cfg(feature = "server")]
use {
    axum::{
        extract::{Query, State},
        http::StatusCode,
        response::Json,
        routing::get,
        Router,
    },
    serde::{Deserialize, Serialize},
    tower_http::cors::{AllowOrigin, CorsLayer},
};

#[cfg(feature = "server")]
use {
    crate::{array, maybe_bs58_aware_lowercase, parse_pubkey},
    rand::{self, Rng},
    sha2::{Digest, Sha256},
    solana_pubkey::Pubkey,
    std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        time::{Duration, Instant},
    },
    tokio::sync::{OwnedSemaphorePermit, Semaphore},
};

#[cfg(feature = "server")]
#[derive(Debug, Deserialize)]
pub struct GrindQuery {
    pub base: String,
    pub suffix: Option<String>,
    /// Program that will own the created account. Defaults to
    /// `VANITY_DEFAULT_TOKEN_PROGRAM`. Token-2022 mints must pass
    /// `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`, because the owner is
    /// hashed into the derived address.
    pub owner: Option<String>,
}

#[cfg(feature = "server")]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GrindResult {
    pub address: String,
    pub seed: String,
    pub seed_bytes: Vec<u8>,
    pub base: String,
    pub owner: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub case_insensitive: bool,
    pub attempts: u64,
    pub duration_seconds: f64,
    pub attempts_per_second: u64,
}

#[cfg(feature = "server")]
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<ServerConfig>,
    slots: Arc<Semaphore>,
    start_rate: Arc<Mutex<StartRate>>,
}

// Covers the existing omni, yLP and hLP suffixes. Count both constraints so a
// configured prefix cannot silently multiply the permitted request complexity.
const MAX_TARGET_LENGTH: usize = 4;
type ApiError = (StatusCode, Json<serde_json::Value>);

#[derive(Debug)]
struct ServerConfig {
    owner: Pubkey,
    prefix: Option<String>,
    case_insensitive: bool,
    timeout: Duration,
    max_concurrent_tasks: usize,
    requests_per_minute: u32,
}

impl ServerConfig {
    fn from_env() -> Result<Self, String> {
        let owner = parse_pubkey(
            &std::env::var("VANITY_DEFAULT_TOKEN_PROGRAM")
                .unwrap_or_else(|_| "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()),
        )
        .map_err(|e| format!("Invalid VANITY_DEFAULT_TOKEN_PROGRAM: {e}"))?;
        let case_insensitive = std::env::var("VANITY_DEFAULT_CASE_INSENSITIVE")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .map_err(|_| "VANITY_DEFAULT_CASE_INSENSITIVE must be true or false".to_string())?;
        let prefix = validate_target(
            &std::env::var("VANITY_DEFAULT_PREFIX").ok(),
            case_insensitive,
        )
        .map_err(|e| format!("Invalid VANITY_DEFAULT_PREFIX: {e}"))?;
        Ok(Self {
            owner,
            prefix,
            case_insensitive,
            timeout: Duration::from_secs(bounded_env("VANITY_TASK_TIMEOUT", 30, 300)? as u64),
            max_concurrent_tasks: bounded_env("VANITY_MAX_CONCURRENT_TASKS", 1, 64)? as usize,
            requests_per_minute: bounded_env("VANITY_MAX_REQUESTS_PER_MINUTE", 60, 10_000)?,
        })
    }
}

fn bounded_env(name: &str, default: u32, max: u32) -> Result<u32, String> {
    match std::env::var(name) {
        Ok(value) => parse_limit(name, &value, max),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(_) => Err(format!("{name} must be an integer between 1 and {max}")),
    }
}

fn parse_limit(name: &str, value: &str, max: u32) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=max).contains(n))
        .ok_or_else(|| format!("{name} must be an integer between 1 and {max}"))
}

// A process-wide token bucket has bounded memory and does not trust client-
// supplied forwarding headers. Per-client fairness belongs at a trusted proxy.
#[derive(Debug)]
struct StartRate {
    tokens: f64,
    last_refill: Instant,
}

impl StartRate {
    fn allow(&mut self, now: Instant, per_minute: u32) -> bool {
        let capacity = f64::from(per_minute);
        self.tokens = (self.tokens
            + now.duration_since(self.last_refill).as_secs_f64() * capacity / 60.0)
            .min(capacity);
        self.last_refill = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(feature = "server")]
impl AppState {
    fn new(config: ServerConfig) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(config.max_concurrent_tasks)),
            start_rate: Arc::new(Mutex::new(StartRate {
                tokens: f64::from(config.requests_per_minute),
                last_refill: Instant::now(),
            })),
            config: Arc::new(config),
        }
    }

    fn admit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        // Reject rather than building an unbounded queue of waiting requests.
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "Grinding capacity is busy; retry later",
            )
        })?;
        if !self
            .start_rate
            .lock()
            .expect("start rate mutex poisoned")
            .allow(Instant::now(), self.config.requests_per_minute)
        {
            return Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "Grinding start rate exceeded; retry later",
            ));
        }
        Ok(permit)
    }
}

fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({ "error": message.into() })))
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct WorkBudget {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, PartialEq)]
enum GrindError {
    TimedOut,
    Cancelled,
}

#[cfg(feature = "server")]
fn configure_cors() -> CorsLayer {
    let cors_origins = std::env::var("VANITY_CORS_ORIGINS").ok();

    if let Some(origins_str) = cors_origins {
        // Parse comma-separated origins
        let origins: Vec<String> = origins_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Parse origins into HeaderValues, filtering out invalid ones
        let origin_list: Vec<axum::http::HeaderValue> =
            origins.iter().filter_map(|s| s.parse().ok()).collect();

        if origin_list.is_empty() {
            // No valid origins, use permissive CORS
            CorsLayer::permissive()
        } else {
            // Build CORS layer with allowed origins
            let mut cors = CorsLayer::new()
                .allow_methods(tower_http::cors::AllowMethods::any())
                .allow_headers(tower_http::cors::AllowHeaders::any());

            // Add origins - use exact for single, list for multiple
            if origin_list.len() == 1 {
                cors = cors.allow_origin(AllowOrigin::exact(origin_list[0].clone()));
            } else {
                cors = cors.allow_origin(AllowOrigin::list(origin_list));
            }

            cors
        }
    } else {
        // No CORS config, use permissive CORS
        CorsLayer::permissive()
    }
}

#[cfg(feature = "server")]
pub async fn start_server(args: crate::ServerArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file
    dotenvy::dotenv().ok(); // Ignore errors if .env file doesn't exist

    // Determine port: CLI arg > env var > default
    let port = args
        .port
        .or_else(|| {
            std::env::var("VANITY_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(8080);

    let app_state = AppState::new(ServerConfig::from_env()?);

    let app = router(app_state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("🚀 Vanity server running on http://0.0.0.0:{}", port);
    println!("📖 API Documentation:");
    println!("  GET  / - API documentation");
    println!("  GET  /grind - Grind vanity addresses (base required as query param)");
    println!("  GET  /health - Health check");

    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(feature = "server")]
async fn root() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "vanity-server",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "A blazingly fast tool for grinding vanity addresses on Solana",
        "endpoints": {
            "GET /": "API documentation (this endpoint)",
            "GET /health": "Health check",
            "GET /grind": "Grind vanity addresses synchronously"
        },
        "configuration": {
            "note": "Base is required as a query parameter. Other parameters can be configured via environment variables.",
            "required_vars": [],
            "optional_vars": [
                "VANITY_DEFAULT_TOKEN_PROGRAM",
                "VANITY_DEFAULT_PREFIX",
                "VANITY_DEFAULT_CASE_INSENSITIVE",
                "VANITY_TASK_TIMEOUT",
                "VANITY_MAX_CONCURRENT_TASKS",
                "VANITY_MAX_REQUESTS_PER_MINUTE",
                "VANITY_PORT",
                "VANITY_CORS_ORIGINS"
            ],
            "query_params": [
                "base - Base pubkey for grinding (required)",
                "suffix - Target suffix (optional; prefix + suffix must total at most 4 Base58 characters)",
                "owner - Program that will own the created account (optional; defaults to VANITY_DEFAULT_TOKEN_PROGRAM). Token-2022 mints pass TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
            ]
        },
        "example_usage": {
            "curl": "curl -X GET 'http://localhost:8080/grind?base=3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd&suffix=omni'",
            "curl_token_2022": "curl -X GET 'http://localhost:8080/grind?base=3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd&suffix=yLP&owner=TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'",
            "description": "Returns a matching address within the configured time budget, or a controlled error"
        },
        "response_format": {
            "address": "Generated vanity address",
            "seed": "Seed used to generate the address (string)",
            "seed_bytes": "Seed used to generate the address (byte array)",
            "base": "Base pubkey used",
            "owner": "Owner pubkey used",
            "prefix": "Target prefix (if specified)",
            "suffix": "Target suffix (if specified)",
            "case_insensitive": "Whether case-insensitive matching was used",
            "attempts": "Number of attempts made",
            "duration_seconds": "Time taken in seconds",
            "attempts_per_second": "Performance metric"
        }
    }))
}

#[cfg(feature = "server")]
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "vanity-server",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

#[cfg(feature = "server")]
async fn grind_sync(
    Query(query): Query<GrindQuery>,
    State(state): State<AppState>,
) -> Result<Json<GrindResult>, ApiError> {
    let config = &state.config;
    let base = parse_pubkey(&query.base)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, format!("Invalid base pubkey: {e}")))?;
    let owner = match query.owner.filter(|owner| !owner.trim().is_empty()) {
        Some(owner) => parse_pubkey(&owner).map_err(|e| {
            api_error(
                StatusCode::BAD_REQUEST,
                format!("Invalid owner pubkey: {e}"),
            )
        })?,
        None => config.owner,
    };
    let suffix = validate_target(&query.suffix, config.case_insensitive)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let prefix = config.prefix.clone();
    if prefix.as_deref().unwrap_or("").len() + suffix.as_deref().unwrap_or("").len()
        > MAX_TARGET_LENGTH
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Prefix and suffix together must not exceed 4 Base58 characters",
        ));
    }

    let permit = state.admit()?;
    let cancelled = Arc::new(AtomicBool::new(false));
    // Dropping an HTTP handler does not stop spawn_blocking. Signal the worker
    // explicitly on disconnect, handler cancellation, or an elapsed timeout.
    let _cancel_on_drop = CancelOnDrop(cancelled.clone());
    let budget = WorkBudget {
        deadline: Instant::now() + config.timeout,
        cancelled,
    };
    let case_insensitive = config.case_insensitive;
    let worker = tokio::task::spawn_blocking(move || {
        // Capacity is released only when computation actually stops, even if
        // the HTTP handler has already returned or has been dropped.
        let _permit = permit;
        grind_sync_blocking(base, owner, prefix, suffix, case_insensitive, budget)
    });

    match tokio::time::timeout(config.timeout, worker).await {
        Ok(Ok(Ok(result))) => Ok(Json(result)),
        Err(_) | Ok(Ok(Err(GrindError::TimedOut))) => Err(api_error(
            StatusCode::GATEWAY_TIMEOUT,
            "Grinding time budget exceeded",
        )),
        Ok(Ok(Err(GrindError::Cancelled))) => Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Grinding was cancelled",
        )),
        Ok(Err(e)) => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Grinding task failed: {e}"),
        )),
    }
}

#[cfg(feature = "server")]
fn grind_sync_blocking(
    base: Pubkey,
    owner: Pubkey,
    prefix: Option<String>,
    suffix: Option<String>,
    case_insensitive: bool,
    budget: WorkBudget,
) -> Result<GrindResult, GrindError> {
    let start_time = Instant::now();
    let prefix_str = prefix.as_deref().unwrap_or("").to_string();
    let suffix_str = suffix.as_deref().unwrap_or("").to_string();
    let base_sha = Sha256::new().chain_update(base);
    let mut count = 0_u64;

    loop {
        // Check before starting and at bounded intervals while hashing. The
        // deadline is enforced here as well as in the async HTTP handler.
        if count % 1024 == 0 {
            if budget.cancelled.load(Ordering::Acquire) {
                return Err(GrindError::Cancelled);
            }
            if Instant::now() >= budget.deadline {
                return Err(GrindError::TimedOut);
            }
        }

        let mut seed_iter = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(16);
        let seed: [u8; 16] = array::from_fn(|_| seed_iter.next().unwrap());

        let pubkey_bytes: [u8; 32] = base_sha
            .clone()
            .chain_update(seed)
            .chain_update(owner)
            .finalize()
            .into();
        let pubkey = fd_bs58::encode_32(pubkey_bytes);
        let out_str_target_check = maybe_bs58_aware_lowercase(&pubkey, case_insensitive);

        count += 1;

        if out_str_target_check.starts_with(&prefix_str)
            && out_str_target_check.ends_with(&suffix_str)
        {
            let time_secs = start_time.elapsed().as_secs_f64();
            return Ok(GrindResult {
                address: pubkey,
                seed: String::from_utf8_lossy(&seed).to_string(),
                seed_bytes: seed.to_vec(),
                base: base.to_string(),
                owner: owner.to_string(),
                prefix,
                suffix,
                case_insensitive,
                attempts: count,
                duration_seconds: time_secs,
                attempts_per_second: (count as f64 / time_secs) as u64,
            });
        }
    }
}

#[cfg(feature = "server")]
fn validate_target(
    target: &Option<String>,
    case_insensitive: bool,
) -> Result<Option<String>, String> {
    const BS58_CHARS: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

    if let Some(ref target_str) = target {
        if target_str.len() > MAX_TARGET_LENGTH {
            return Err("Target must not exceed 4 Base58 characters".to_string());
        }
        for c in target_str.chars() {
            if !BS58_CHARS.contains(c) {
                return Err("Target must contain only Base58 characters".to_string());
            }
        }
        Ok(Some(maybe_bs58_aware_lowercase(
            target_str,
            case_insensitive,
        )))
    } else {
        Ok(None)
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .route("/grind", get(grind_sync))
        .layer(configure_cors())
        .with_state(state)
}

#[cfg(test)]
mod tests;
