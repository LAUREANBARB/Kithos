use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kithos_proto::{
    normalize_username, validate_username, AuthResponse, ErrorBody, LoginRequest, PASSWORD_MIN,
    RegisterRequest, User, DISPLAY_NAME_MAX,
};
use sqlx::sqlite::SqlitePool;
use sqlx::Row;

use crate::auth::{hash_password, hash_token, new_session_token, verify_password, SESSION_TTL_SECONDS};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/me", get(me))
        .with_state(state)
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { error: self.message })).into_response()
    }
}

async fn health() -> &'static str {
    "ok"
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let username = normalize_username(&body.username);
    validate_username(&username).map_err(|e| {
        ApiError::new(StatusCode::BAD_REQUEST, format!("invalid username: {e:?}"))
    })?;

    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() || display_name.chars().count() > DISPLAY_NAME_MAX {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid display name"));
    }
    if body.password.chars().count() < PASSWORD_MIN {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }

    let password_hash = hash_password(&body.password)
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "hashing failed"))?;
    let now = now_secs();

    let inserted = sqlx::query_scalar::<_, i64>(
        "insert into users (username, display_name, password_hash, created_at)
         values (?, ?, ?, ?) returning id",
    )
    .bind(&username)
    .bind(&display_name)
    .bind(&password_hash)
    .bind(now)
    .fetch_one(&state.pool)
    .await;

    let id = match inserted {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::new(StatusCode::CONFLICT, "username taken"));
        }
        Err(_) => {
            return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "could not create user"));
        }
    };

    let token = create_session(&state.pool, id).await?;

    Ok(Json(AuthResponse {
        token,
        user: User { id, username, display_name, bio: String::new() },
    }))
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let username = normalize_username(&body.username);

    let row = sqlx::query(
        "select id, username, display_name, bio, password_hash from users where username = ?",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "lookup failed"))?;

    let Some(row) = row else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid credentials"));
    };

    let stored: String = row.try_get("password_hash").unwrap_or_default();
    if !verify_password(&body.password, &stored) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid credentials"));
    }

    let id: i64 = row.try_get("id").unwrap_or_default();
    let token = create_session(&state.pool, id).await?;

    Ok(Json(AuthResponse {
        token,
        user: User {
            id,
            username: row.try_get("username").unwrap_or_default(),
            display_name: row.try_get("display_name").unwrap_or_default(),
            bio: row.try_get("bio").unwrap_or_default(),
        },
    }))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<User>, ApiError> {
    let token = bearer_token(&headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing token"))?;
    let token_hash = hash_token(token);

    let row = sqlx::query(
        "select u.id, u.username, u.display_name, u.bio
         from sessions s join users u on u.id = s.user_id
         where s.token_hash = ? and s.expires_at > ?",
    )
    .bind(&token_hash)
    .bind(now_secs())
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "lookup failed"))?;

    let Some(row) = row else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid or expired token"));
    };

    Ok(Json(User {
        id: row.try_get("id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        display_name: row.try_get("display_name").unwrap_or_default(),
        bio: row.try_get("bio").unwrap_or_default(),
    }))
}

async fn create_session(pool: &SqlitePool, user_id: i64) -> Result<String, ApiError> {
    let token = new_session_token();
    let now = now_secs();

    sqlx::query(
        "insert into sessions (token_hash, user_id, created_at, expires_at) values (?, ?, ?, ?)",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(now)
    .bind(now + SESSION_TTL_SECONDS)
    .execute(pool)
    .await
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "session failed"))?;

    Ok(token)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim).filter(|t| !t.is_empty())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
