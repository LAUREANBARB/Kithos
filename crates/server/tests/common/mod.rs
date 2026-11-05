#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use kithos_server::{app, init_memory_db, AppState};
use serde_json::Value;
use sqlx::sqlite::SqlitePool;
use tower::ServiceExt;

pub const PROTOCOL: &str = "1";

pub async fn setup() -> (Router, SqlitePool) {
    let pool = init_memory_db().await.expect("in-memory database");
    (app(AppState::new(pool.clone())), pool)
}

pub async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-kithos-protocol", PROTOCOL);

    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }

    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

pub async fn register(router: &Router, username: &str, password: &str) -> (String, i64) {
    let body = serde_json::json!({
        "username": username,
        "display_name": "Test User",
        "password": password
    });
    let (status, response) = call(router, "POST", "/auth/register", None, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "registration failed: {response}");
    (
        response["token"].as_str().expect("token").to_string(),
        response["user"]["id"].as_i64().expect("user id"),
    )
}

pub async fn first_text_channel(router: &Router, token: &str) -> i64 {
    let (status, guilds) = call(router, "GET", "/guilds", Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "guild listing failed: {guilds}");
    guilds[0]["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .find(|channel| channel["kind"] == "text")
        .and_then(|channel| channel["id"].as_i64())
        .expect("a text channel")
}
