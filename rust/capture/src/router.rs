use std::collections::HashSet;
use std::future::ready;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use axum::{
    routing::{get, post},
    Router,
};
use health::HealthRegistry;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::test_endpoint;
use crate::{sinks, time::TimeSource, v0_endpoint};
use common_redis::Client;
use limiters::redis::RedisLimiter;
use limiters::token_dropper::TokenDropper;

use crate::config::CaptureMode;
use crate::prometheus::{setup_metrics_recorder, track_metrics};

const EVENT_BODY_SIZE: usize = 2 * 1024 * 1024; // 2MB
pub const BATCH_BODY_SIZE: usize = 20 * 1024 * 1024; // 20MB, up from the default 2MB used for normal event payloads
const RECORDING_BODY_SIZE: usize = 25 * 1024 * 1024; // 25MB, up from the default 2MB used for normal event payloads

#[derive(Clone)]
pub struct State {
    pub sink: Arc<dyn sinks::Event + Send + Sync>,
    pub timesource: Arc<dyn TimeSource + Send + Sync>,
    pub redis: Arc<dyn Client + Send + Sync>,
    pub billing_limiter: RedisLimiter,
    pub token_dropper: Arc<TokenDropper>,
    pub event_size_limit: usize,
    pub historical_cfg: HistoricalConfig,
}

#[derive(Clone)]
pub struct HistoricalConfig {
    pub enable_historical_rerouting: bool,
    pub historical_rerouting_threshold_days: i64,
    pub historical_tokens_keys: HashSet<String>,
}

impl HistoricalConfig {
    pub fn new(
        enable_historical_rerouting: bool,
        historical_rerouting_threshold_days: i64,
        tokens_keys: Option<String>,
    ) -> Self {
        let mut htk = HashSet::new();
        if let Some(s) = tokens_keys {
            for entry in s.split(",") {
                htk.insert(entry.trim().to_string());
            }
        }

        HistoricalConfig {
            enable_historical_rerouting,
            historical_rerouting_threshold_days,
            historical_tokens_keys: htk,
        }
    }

    // event_key is one of: "token" "token:ip_addr" or "token:distinct_id"
    // and self.historical_tokens_keys is a set of the same. if the key
    // matches any entry in the set, the event should be rerouted
    pub fn should_reroute(&self, event_key: &str) -> bool {
        // is the event key in the forced_keys list?
        let key_match = self.historical_tokens_keys.contains(event_key);

        // is the token (first component of the event key) in the forced_keys list?
        let token_match = match event_key.split(':').next() {
            Some(token) => !token.is_empty() && self.historical_tokens_keys.contains(token),
            None => false,
        };

        key_match || token_match
    }
}

async fn index() -> &'static str {
    "capture"
}

#[allow(clippy::too_many_arguments)]
pub fn router<
    TZ: TimeSource + Send + Sync + 'static,
    S: sinks::Event + Send + Sync + 'static,
    R: Client + Send + Sync + 'static,
>(
    timesource: TZ,
    liveness: HealthRegistry,
    sink: S,
    redis: Arc<R>,
    billing_limiter: RedisLimiter,
    token_dropper: TokenDropper,
    metrics: bool,
    capture_mode: CaptureMode,
    concurrency_limit: Option<usize>,
    event_size_limit: usize,
    enable_historical_rerouting: bool,
    historical_rerouting_threshold_days: i64,
    historical_tokens_keys: Option<String>,
) -> Router {
    let state = State {
        sink: Arc::new(sink),
        timesource: Arc::new(timesource),
        redis,
        billing_limiter,
        event_size_limit,
        token_dropper: Arc::new(token_dropper),
        historical_cfg: HistoricalConfig::new(
            enable_historical_rerouting,
            historical_rerouting_threshold_days,
            historical_tokens_keys,
        ),
    };

    // Very permissive CORS policy, as old SDK versions
    // and reverse proxies might send funky headers.
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(AllowHeaders::mirror_request())
        .allow_credentials(true)
        .allow_origin(AllowOrigin::mirror_request());

    let test_router = Router::new()
        .route(
            "/test/black_hole",
            post(test_endpoint::test_black_hole)
                .get(test_endpoint::test_black_hole)
                .options(v0_endpoint::options),
        )
        .route(
            "/test/black_hole/",
            post(test_endpoint::test_black_hole)
                .get(test_endpoint::test_black_hole)
                .options(v0_endpoint::options),
        )
        .layer(DefaultBodyLimit::max(BATCH_BODY_SIZE));

    let batch_router = Router::new()
        .route(
            "/batch",
            post(v0_endpoint::event)
                .get(v0_endpoint::event)
                .options(v0_endpoint::options),
        )
        .route(
            "/batch/",
            post(v0_endpoint::event)
                .get(v0_endpoint::event)
                .options(v0_endpoint::options),
        )
        .layer(DefaultBodyLimit::max(BATCH_BODY_SIZE)); // Have to use this, rather than RequestBodyLimitLayer, because we use `Bytes` in the handler (this limit applies specifically to Bytes body types)

    let event_router = Router::new()
        .route(
            "/e",
            post(v0_endpoint::event)
                .get(v0_endpoint::event)
                .options(v0_endpoint::options),
        )
        .route(
            "/e/",
            post(v0_endpoint::event)
                .get(v0_endpoint::event)
                .options(v0_endpoint::options),
        )
        .route(
            "/i/v0/e",
            post(v0_endpoint::event)
                .get(v0_endpoint::event)
                .options(v0_endpoint::options),
        )
        .route(
            "/i/v0/e/",
            post(v0_endpoint::event)
                .get(v0_endpoint::event)
                .options(v0_endpoint::options),
        )
        .route("/i/v0", get(index))
        .route("/i/v0/", get(index))
        .layer(DefaultBodyLimit::max(EVENT_BODY_SIZE));

    let status_router = Router::new()
        .route("/", get(index))
        .route("/_readiness", get(index))
        .route("/_liveness", get(move || ready(liveness.get_status())));

    let recordings_router = Router::new()
        .route(
            "/s",
            post(v0_endpoint::recording)
                .get(v0_endpoint::recording)
                .options(v0_endpoint::options),
        )
        .route(
            "/s/",
            post(v0_endpoint::recording)
                .get(v0_endpoint::recording)
                .options(v0_endpoint::options),
        )
        .layer(DefaultBodyLimit::max(RECORDING_BODY_SIZE));

    let mut router = match capture_mode {
        CaptureMode::Events => Router::new()
            .merge(batch_router)
            .merge(event_router)
            .merge(test_router),
        CaptureMode::Recordings => Router::new().merge(recordings_router),
    };

    if let Some(limit) = concurrency_limit {
        router = router.layer(ConcurrencyLimitLayer::new(limit));
    }

    let router = router
        .merge(status_router)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(axum::middleware::from_fn(track_metrics))
        .with_state(state);

    // Don't install metrics unless asked to
    // Installing a global recorder when capture is used as a library (during tests etc)
    // does not work well.
    if metrics {
        let recorder_handle = setup_metrics_recorder();
        router.route("/metrics", get(move || ready(recorder_handle.render())))
    } else {
        router
    }
}
