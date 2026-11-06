use super::*;
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

const BASE: &str = "11111111111111111111111111111111";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

fn config() -> ServerConfig {
    ServerConfig {
        owner: parse_pubkey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(),
        prefix: None,
        case_insensitive: false,
        timeout: Duration::from_secs(3),
        max_concurrent_tasks: 1,
        requests_per_minute: 60,
    }
}

fn query(suffix: Option<&str>) -> GrindQuery {
    GrindQuery {
        base: BASE.to_string(),
        suffix: suffix.map(str::to_string),
        owner: None,
    }
}

// Test-only impossible target makes lifecycle tests deterministic, independent
// of random luck. Real startup configuration rejects this non-Base58 prefix.
fn never_matching_config() -> ServerConfig {
    ServerConfig {
        prefix: Some("0".to_string()),
        ..config()
    }
}

struct TestServer {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start(state: AppState) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        Self { address, task }
    }

    async fn start_request(&self, path: &str) -> TcpStream {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        stream
    }

    async fn get(&self, path: &str) -> String {
        let mut stream = self.start_request(path).await;
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        response
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn wait_for_slots(state: &AppState, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while state.slots.available_permits() != expected {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("worker did not acquire/release capacity promptly");
}

#[test]
fn validates_target_alphabet_length_and_supported_suffixes() {
    for suffix in ["omni", "yLP", "hLP", ""] {
        assert_eq!(
            validate_target(&Some(suffix.to_string()), false).unwrap(),
            Some(suffix.to_string())
        );
    }
    assert_eq!(validate_target(&None, false).unwrap(), None);
    assert_eq!(
        validate_target(&Some("yLP".to_string()), true)
            .unwrap()
            .as_deref(),
        Some("yLp")
    );
    for suffix in ["abcde", "0", "O", "I", "l", "é", "a b"] {
        assert!(validate_target(&Some(suffix.to_string()), false).is_err());
    }
    assert!(validate_target(&Some("a".repeat(45)), false).is_err());
}

#[test]
fn resource_limits_cannot_be_disabled_or_overflow() {
    for invalid in ["0", "-1", "301", "4294967296", "unlimited", ""] {
        assert!(parse_limit("TEST_LIMIT", invalid, 300).is_err());
    }
    assert_eq!(parse_limit("TEST_LIMIT", "1", 300).unwrap(), 1);
    assert_eq!(parse_limit("TEST_LIMIT", "300", 300).unwrap(), 300);
}

#[test]
fn rate_limit_refills_without_accumulating_an_unbounded_burst() {
    let start = Instant::now();
    let mut rate = StartRate {
        tokens: 2.0,
        last_refill: start,
    };
    assert!(rate.allow(start, 2));
    assert!(rate.allow(start, 2));
    assert!(!rate.allow(start, 2));
    assert!(!rate.allow(start + Duration::from_secs(29), 2));
    assert!(rate.allow(start + Duration::from_secs(30), 2));
    assert!(!rate.allow(start + Duration::from_secs(30), 2));
    assert!(rate.allow(start + Duration::from_secs(300), 2));
    assert!(rate.allow(start + Duration::from_secs(300), 2));
    assert!(!rate.allow(start + Duration::from_secs(300), 2));
}

#[tokio::test]
async fn rejects_invalid_input_before_admitting_work() {
    let state = AppState::new(config());
    let server = TestServer::start(state.clone()).await;
    for params in [
        format!("base={BASE}&suffix={}", "a".repeat(45)),
        format!("base={BASE}&suffix=0"),
        format!("base={BASE}&owner=not-a-pubkey"),
        "base=not-a-pubkey".to_string(),
    ] {
        assert!(server
            .get(&format!("/grind?{params}"))
            .await
            .starts_with("HTTP/1.1 400"));
    }
    assert_eq!(state.slots.available_permits(), 1);
    assert_eq!(state.start_rate.lock().unwrap().tokens, 60.0);

    let state = AppState::new(ServerConfig {
        prefix: Some("ab".to_string()),
        ..config()
    });
    let error = grind_sync(Query(query(Some("abc"))), State(state.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(state.start_rate.lock().unwrap().tokens, 60.0);
}

#[tokio::test]
async fn preserves_default_and_token_2022_address_derivation() {
    let state = AppState::new(config());
    for owner in [None, Some(TOKEN_2022.to_string())] {
        let expected_owner = owner
            .as_ref()
            .map(|s| parse_pubkey(s).unwrap())
            .unwrap_or(state.config.owner);
        let Json(result) = grind_sync(
            Query(GrindQuery {
                owner,
                ..query(Some("a"))
            }),
            State(state.clone()),
        )
        .await
        .unwrap();
        assert!(result.address.ends_with('a'));
        assert_eq!(result.owner, expected_owner.to_string());
        assert_eq!(
            Pubkey::create_with_seed(&parse_pubkey(BASE).unwrap(), &result.seed, &expected_owner)
                .unwrap()
                .to_string(),
            result.address
        );
        assert_eq!(result.seed.as_bytes(), result.seed_bytes);
        assert!(result.attempts > 0);
        assert_eq!(state.slots.available_permits(), 1);
    }
}

#[tokio::test]
async fn returns_429_without_queueing_when_capacity_is_full() {
    let state = AppState::new(config());
    let permit = state.slots.clone().try_acquire_owned().unwrap();
    let server = TestServer::start(state.clone()).await;
    assert!(server
        .get(&format!("/grind?base={BASE}"))
        .await
        .starts_with("HTTP/1.1 429"));
    assert!(server.get("/health").await.starts_with("HTTP/1.1 200"));
    assert_eq!(state.start_rate.lock().unwrap().tokens, 60.0);
    drop(permit);
    assert!(server
        .get(&format!("/grind?base={BASE}"))
        .await
        .starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn limits_repeated_fast_jobs_even_when_capacity_is_available() {
    let state = AppState::new(ServerConfig {
        requests_per_minute: 2,
        ..config()
    });
    let server = TestServer::start(state.clone()).await;
    for _ in 0..2 {
        assert!(server
            .get(&format!("/grind?base={BASE}"))
            .await
            .starts_with("HTTP/1.1 200"));
    }
    assert!(server
        .get(&format!("/grind?base={BASE}"))
        .await
        .starts_with("HTTP/1.1 429"));
    assert_eq!(state.slots.available_permits(), 1);
    assert!(server.get("/health").await.starts_with("HTTP/1.1 200"));
}

#[test]
fn worker_enforces_its_deadline_without_an_http_timeout() {
    let start = Instant::now();
    let error = grind_sync_blocking(
        parse_pubkey(BASE).unwrap(),
        config().owner,
        None,
        Some("a".repeat(45)),
        false,
        WorkBudget {
            deadline: start + Duration::from_millis(20),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    )
    .unwrap_err();
    assert_eq!(error, GrindError::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn cancelled_or_expired_work_does_not_start_searching() {
    for (cancelled, deadline, expected) in [
        (
            true,
            Instant::now() + Duration::from_secs(3),
            GrindError::Cancelled,
        ),
        (false, Instant::now(), GrindError::TimedOut),
    ] {
        // An unconstrained search would succeed immediately if the initial
        // cancellation/deadline check were missing.
        let result = grind_sync_blocking(
            parse_pubkey(BASE).unwrap(),
            config().owner,
            None,
            None,
            false,
            WorkBudget {
                deadline,
                cancelled: Arc::new(AtomicBool::new(cancelled)),
            },
        );
        assert_eq!(result.unwrap_err(), expected);
    }
}

#[tokio::test]
async fn deadline_returns_504_and_worker_releases_its_slot() {
    let state = AppState::new(ServerConfig {
        timeout: Duration::from_millis(30),
        ..never_matching_config()
    });
    let server = TestServer::start(state.clone()).await;
    assert!(server
        .get(&format!("/grind?base={BASE}"))
        .await
        .starts_with("HTTP/1.1 504"));
    wait_for_slots(&state, 1).await;
    assert!(server.get("/health").await.starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn disconnect_stops_worker_before_its_deadline() {
    let state = AppState::new(never_matching_config());
    let server = TestServer::start(state.clone()).await;
    let stream = server.start_request(&format!("/grind?base={BASE}")).await;
    wait_for_slots(&state, 0).await;
    assert!(server.get("/health").await.starts_with("HTTP/1.1 200"));
    assert!(server
        .get(&format!("/grind?base={BASE}"))
        .await
        .starts_with("HTTP/1.1 429"));
    drop(stream);
    // Must release in <1s, well before the 3s job deadline.
    wait_for_slots(&state, 1).await;
}

#[tokio::test]
async fn aborting_handler_cancels_worker_and_reclaims_capacity() {
    let state = AppState::new(never_matching_config());
    let request = tokio::spawn(grind_sync(Query(query(None)), State(state.clone())));
    wait_for_slots(&state, 0).await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    wait_for_slots(&state, 1).await;
}

#[tokio::test]
async fn cancellation_does_not_release_a_still_running_workers_permit() {
    let state = AppState::new(config());
    let permit = state.admit().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let guard = CancelOnDrop(cancelled.clone());
    let (finish, wait) = std::sync::mpsc::channel();
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        wait.recv_timeout(Duration::from_secs(1)).unwrap();
    });
    drop(guard);
    assert!(cancelled.load(Ordering::Acquire));
    assert_eq!(state.admit().unwrap_err().0, StatusCode::TOO_MANY_REQUESTS);
    finish.send(()).unwrap();
    worker.await.unwrap();
    assert!(state.admit().is_ok());
}
