use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kithos_proto::{
    normalize_username, validate_username, AuthResponse, Channel, ChannelKind, ErrorBody, Event,
    Guild, LoginRequest, Message, MessagePage, RegisterRequest, SendMessageRequest, User,
    DISPLAY_NAME_MAX, MESSAGE_MAX, PAGE_DEFAULT, PAGE_MAX, PASSWORD_MIN, PROTOCOL_HEADER,
    PROTOCOL_VERSION,
};
use serde::Deserialize;
use sqlx::sqlite::SqlitePool;
use sqlx::Row;
use tokio::sync::broadcast;

use crate::auth::{hash_password, hash_token, new_session_token, verify_password, SESSION_TTL_SECONDS};

const EVENT_BUFFER: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub events: broadcast::Sender<Event>,
}

impl AppState {
    pub fn new(pool: SqlitePool) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self { pool, events }
    }
}

pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/me", get(me))
        .route("/guilds", get(guilds))
        .route("/channels/{id}/messages", get(list_messages).post(post_message))
        .route("/events", get(events))
        .route_layer(middleware::from_fn(require_protocol));

    Router::new().route("/health", get(health)).merge(api).with_state(state)
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

async fn require_protocol(request: axum::extract::Request, next: Next) -> Response {
    let supplied = request
        .headers()
        .get(PROTOCOL_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok());

    if supplied == Some(PROTOCOL_VERSION) {
        return next.run(request).await;
    }

    (
        StatusCode::UPGRADE_REQUIRED,
        Json(ErrorBody {
            error: format!("this server speaks protocol {PROTOCOL_VERSION}; send {PROTOCOL_HEADER}"),
        }),
    )
        .into_response()
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

    join_default_guild(&state.pool, id).await?;
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

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<User>, ApiError> {
    Ok(Json(current_user(&state, &headers).await?))
}

async fn guilds(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Guild>>, ApiError> {
    let user = current_user(&state, &headers).await?;

    let rows = sqlx::query(
        "select g.id, g.name from guilds g join members m on m.guild_id = g.id
         where m.user_id = ? order by g.id",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "lookup failed"))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let guild_id: i64 = row.try_get("id").unwrap_or_default();
        let channels = channel_rows(&state.pool, guild_id).await?;
        out.push(Guild {
            id: guild_id,
            name: row.try_get("name").unwrap_or_default(),
            channels,
        });
    }

    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn list_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> Result<Json<MessagePage>, ApiError> {
    let user = current_user(&state, &headers).await?;
    require_membership(&state.pool, user.id, channel_id).await?;

    let limit = query.limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);

    let rows = match query.before {
        Some(before) => {
            sqlx::query(
                "select id, channel_id, author_id, body, created_at from messages
                 where channel_id = ? and id < ? order by id desc limit ?",
            )
            .bind(channel_id)
            .bind(before)
            .bind(limit + 1)
            .fetch_all(&state.pool)
            .await
        }
        None => {
            sqlx::query(
                "select id, channel_id, author_id, body, created_at from messages
                 where channel_id = ? order by id desc limit ?",
            )
            .bind(channel_id)
            .bind(limit + 1)
            .fetch_all(&state.pool)
            .await
        }
    }
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "lookup failed"))?;

    let has_more = rows.len() as i64 > limit;
    let mut messages: Vec<Message> = rows.into_iter().take(limit as usize).map(row_to_message).collect();
    messages.reverse();

    Ok(Json(MessagePage { messages, has_more }))
}

async fn post_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<i64>,
    Json(body): Json<SendMessageRequest>,
) -> Result<Json<Message>, ApiError> {
    let user = current_user(&state, &headers).await?;
    require_membership(&state.pool, user.id, channel_id).await?;

    let text = body.body.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "message is empty"));
    }
    if text.chars().count() > MESSAGE_MAX {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "message too long"));
    }

    let created_at = now_secs();
    let id = sqlx::query_scalar::<_, i64>(
        "insert into messages (channel_id, author_id, body, created_at)
         values (?, ?, ?, ?) returning id",
    )
    .bind(channel_id)
    .bind(user.id)
    .bind(&text)
    .bind(created_at)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "could not store message"))?;

    let message = Message { id, channel_id, author_id: user.id, body: text, created_at };
    let _ = state.events.send(Event::MessageNew(message.clone()));

    Ok(Json(message))
}

async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    current_user(&state, &headers).await?;
    let receiver = state.events.subscribe();
    Ok(ws.on_upgrade(move |socket| crate::ws::run(socket, receiver)))
}

async fn current_user(state: &AppState, headers: &HeaderMap) -> Result<User, ApiError> {
    let token = bearer_token(headers)
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

    Ok(User {
        id: row.try_get("id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        display_name: row.try_get("display_name").unwrap_or_default(),
        bio: row.try_get("bio").unwrap_or_default(),
    })
}

async fn require_membership(pool: &SqlitePool, user_id: i64, channel_id: i64) -> Result<(), ApiError> {
    let row = sqlx::query_scalar::<_, i64>(
        "select 1 from members m join channels c on c.guild_id = m.guild_id
         where m.user_id = ? and c.id = ?",
    )
    .bind(user_id)
    .bind(channel_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "lookup failed"))?;

    match row {
        Some(_) => Ok(()),
        None => Err(ApiError::new(StatusCode::FORBIDDEN, "not a member of this channel")),
    }
}

async fn channel_rows(pool: &SqlitePool, guild_id: i64) -> Result<Vec<Channel>, ApiError> {
    let rows = sqlx::query("select id, guild_id, name, kind from channels where guild_id = ? order by id")
        .bind(guild_id)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "lookup failed"))?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let kind: String = row.try_get("kind").unwrap_or_default();
            Channel {
                id: row.try_get("id").unwrap_or_default(),
                guild_id: row.try_get("guild_id").unwrap_or_default(),
                name: row.try_get("name").unwrap_or_default(),
                kind: if kind == "voice" { ChannelKind::Voice } else { ChannelKind::Text },
            }
        })
        .collect())
}

async fn join_default_guild(pool: &SqlitePool, user_id: i64) -> Result<(), ApiError> {
    let existing = sqlx::query_scalar::<_, i64>("select id from guilds order by id limit 1")
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "guild lookup failed"))?;

    let (guild_id, role) = match existing {
        Some(id) => {
            let members: i64 = sqlx::query_scalar("select count(*) from members where guild_id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap_or_default();
            (id, if members == 0 { "admin" } else { "member" })
        }
        None => {
            let id = sqlx::query_scalar::<_, i64>(
                "insert into guilds (name) values ('Kithos') returning id",
            )
            .fetch_one(pool)
            .await
            .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "guild creation failed"))?;

            sqlx::query(
                "insert into channels (guild_id, name, kind)
                 values (?, 'general', 'text'), (?, 'voice', 'voice')",
            )
            .bind(id)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "channel creation failed"))?;

            (id, "admin")
        }
    };

    sqlx::query("insert into members (guild_id, user_id, role) values (?, ?, ?)")
        .bind(guild_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "membership failed"))?;

    Ok(())
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

fn row_to_message(row: sqlx::sqlite::SqliteRow) -> Message {
    Message {
        id: row.try_get("id").unwrap_or_default(),
        channel_id: row.try_get("channel_id").unwrap_or_default(),
        author_id: row.try_get("author_id").unwrap_or_default(),
        body: row.try_get("body").unwrap_or_default(),
        created_at: row.try_get("created_at").unwrap_or_default(),
    }
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
