use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kithos_server::{app, init_memory_db, AppState};
use tower::ServiceExt;

async fn router() -> axum::Router {
    let pool = init_memory_db().await.expect("in-memory database");
    app(AppState { pool })
}

async fn register(router: &axum::Router, username: &str, password: &str) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "username": username,
        "display_name": "Test User",
        "password": password
    });
    let request = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn register_returns_a_token_and_user() {
    let app = router().await;
    let (status, body) = register(&app, "Dvrkxn", "correct horse battery").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].as_str().is_some_and(|t| t.len() >= 32));
    assert_eq!(body["user"]["username"], "dvrkxn");
    assert_eq!(body["user"]["display_name"], "Test User");
}

#[tokio::test]
async fn register_rejects_duplicate_usernames_case_insensitively() {
    let app = router().await;
    let (first, _) = register(&app, "dvrkxn", "correct horse battery").await;
    let (second, body) = register(&app, "DVRKXN", "another good password").await;

    assert_eq!(first, StatusCode::OK);
    assert_eq!(second, StatusCode::CONFLICT);
    assert_eq!(body["error"], "username taken");
}

#[tokio::test]
async fn register_rejects_short_passwords() {
    let app = router().await;
    let (status, body) = register(&app, "someone", "short").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "password too short");
}

#[tokio::test]
async fn login_rejects_a_wrong_password() {
    let app = router().await;
    register(&app, "dvrkxn", "correct horse battery").await;

    let body = serde_json::json!({ "username": "dvrkxn", "password": "wrong horse battery" });
    let request = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_rejects_missing_and_invalid_tokens() {
    let app = router().await;

    let missing = Request::builder().uri("/me").body(Body::empty()).unwrap();
    assert_eq!(app.clone().oneshot(missing).await.unwrap().status(), StatusCode::UNAUTHORIZED);

    let invalid = Request::builder()
        .uri("/me")
        .header("authorization", "Bearer not-a-real-token")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(invalid).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_returns_the_authenticated_user() {
    let app = router().await;
    let (_, body) = register(&app, "dvrkxn", "correct horse battery").await;
    let token = body["token"].as_str().expect("token").to_string();

    let request = Request::builder()
        .uri("/me")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let user: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(user["username"], "dvrkxn");
    assert_eq!(user["bio"], "");
}
