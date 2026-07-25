use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Query, Request},
    http::{header::HeaderValue, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::calc::{build_report, CheckReport, Machine};
use crate::{machine, resolve};

#[derive(Deserialize)]
struct CheckQuery {
    model: String,
    total: Option<f64>,
    os: Option<f64>,
    docker: Option<f64>,
}

pub async fn run(host: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(healthz))
        .route("/healthz", get(healthz))
        .route("/api/check", get(check))
        .route("/api/machine", get(machine_info))
        .layer(middleware::from_fn(signature_headers));
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    println!("mulaim web app listening on http://{host}:{port}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../assets/index.html"))
}

async fn healthz() -> &'static str {
    "ok"
}

/// Every response carries the operator signature (ASCII-only, per HTTP rules).
async fn signature_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    res.headers_mut().insert(
        "x-powered-by",
        HeaderValue::from_static("Mulaim - LEAP RD&O (https://leap.sa)"),
    );
    res
}

/// Report this machine's memory — but only to loopback clients, so a hosted
/// deployment never presents the server's hardware as the visitor's. Requests
/// arriving through a reverse proxy (forwarding headers present) are treated
/// as remote even when the proxy connects from loopback.
async fn machine_info(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Json<Value> {
    let proxied = headers.contains_key("x-forwarded-for") || headers.contains_key("x-real-ip");
    if !peer.ip().is_loopback() || proxied {
        return Json(json!({ "detected": false, "reason": "remote_client" }));
    }
    match tokio::task::spawn_blocking(machine::detect).await.ok().flatten() {
        Some(m) => Json(json!({ "detected": true, "machine": m })),
        None => Json(json!({ "detected": false, "reason": "unsupported_platform" })),
    }
}

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, code: &str, message: &str) -> ApiError {
    (status, Json(json!({ "error": code, "message_en": message })))
}

async fn check(Query(q): Query<CheckQuery>) -> Result<Json<CheckReport>, ApiError> {
    let mut machine = Machine::default();
    if let Some(t) = q.total {
        machine.total_unified_gb = t;
    }
    if let Some(o) = q.os {
        machine.os_reserved_gb = o;
    }
    if let Some(d) = q.docker {
        machine.docker_reserved_gb = d;
    }

    let sane = machine.total_unified_gb.is_finite()
        && machine.os_reserved_gb.is_finite()
        && machine.docker_reserved_gb.is_finite()
        && machine.total_unified_gb > 0.0
        && machine.total_unified_gb <= 4096.0
        && machine.os_reserved_gb >= 0.0
        && machine.docker_reserved_gb >= 0.0;
    if !sane {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "bad_machine",
            "Machine memory values are out of range.",
        ));
    }

    let Some(model) = resolve::sanitize_model_name(&q.model) else {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "bad_model",
            "Model name is empty, too long, or contains unsupported characters.",
        ));
    };

    let input = model.clone();
    let resolution = tokio::task::spawn_blocking(move || resolve::resolve(&model))
        .await
        .map_err(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Resolution task failed.",
            )
        })?;

    let Some(params_b) = resolution.params_b else {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unresolved",
            "Could not infer the parameter count. Use a name that includes the size (Qwen3-14B) or pass it directly (14b).",
        ));
    };

    Ok(Json(build_report(&input, &resolution, params_b, &machine)))
}
