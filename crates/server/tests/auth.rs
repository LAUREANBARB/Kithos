mod common;

use axum::http::{Request, StatusCode};
use common::{call, register, setup, PROTOCOL};
use axum::body::Body;
use tower::ServiceExt;

#[tokio::test]
async fn register_returns_a_token_and_user() {
    let (app, _) = setup().await;
    let (status, body) = call(
        &app,
        "POST",
        "/auth/register",
        None,
        Some(serde_json::json!({
            "username": "Dvrkxn",
            "display_name": "Test User",
            "password": "correct horse battery"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].as_str().is_some_and(|t| t.len() >= 32));
    assert_eq!(body["user"]["username"], "dvrkxn");
    assert_eq!(body["user"]["display_name"], "Test User");
}

#[tokio::test]
async fn register_rejects_duplicate_usernames_case_insensitively() {
    let (app, _) = setup().await;
    register(&app, "dvrkxn", "correct horse battery").await;

    let (status, body) = call(
        &app,
        "POST",
        "/auth/register",
        None,
        Some(serde_json::json!({
            "username": "DVRKXN",
            "display_name": "Second",
            "password": "another good password"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "username taken");
}

#[tokio::test]
async fn register_rejects_short_passwords() {
    let (app, _) = setup().await;
    let (status, body) = call(
        &app,
        "POST",
        "/auth/register",
        None,
        Some(serde_json::json!({
            "username": "someone",
            "display_name": "Someone",
            "password": "short"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "password too short");
}

#[tokio::test]
async fn login_rejects_a_wrong_password() {
    let (app, _) = setup().await;
    register(&app, "dvrkxn", "correct horse battery").await;

    let (status, _) = call(
        &app,
        "POST",
        "/auth/login",
        None,
        Some(serde_json::json!({ "username": "dvrkxn", "password": "wrong horse battery" })),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_rejects_missing_and_invalid_tokens() {
    let (app, _) = setup().await;

    let (missing, _) = call(&app, "GET", "/me", None, None).await;
    assert_eq!(missing, StatusCode::UNAUTHORIZED);

    let (invalid, _) = call(&app, "GET", "/me", Some("not-a-real-token"), None).await;
    assert_eq!(invalid, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_returns_the_authenticated_user() {
    let (app, _) = setup().await;
    let (token, _) = register(&app, "dvrkxn", "correct horse battery").await;

    let (status, user) = call(&app, "GET", "/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["username"], "dvrkxn");
    assert_eq!(user["bio"], "");
}

#[tokio::test]
async fn requests_without_a_protocol_version_are_refused() {
    let (app, _) = setup().await;

    let request = Request::builder()
        .uri("/me")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn an_old_protocol_version_is_refused_with_the_current_one_named() {
    let (app, _) = setup().await;

    let request = Request::builder()
        .uri("/me")
        .header("x-kithos-protocol", "0")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["error"].as_str().unwrap().contains(PROTOCOL));
}
