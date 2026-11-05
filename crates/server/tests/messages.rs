mod common;

use axum::http::StatusCode;
use common::{call, first_text_channel, register, setup};

#[tokio::test]
async fn registration_creates_a_guild_with_a_text_and_a_voice_channel() {
    let (app, _) = setup().await;
    let (token, _) = register(&app, "dvrkxn", "correct horse battery").await;

    let (status, guilds) = call(&app, "GET", "/guilds", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);

    let guild = &guilds[0];
    assert_eq!(guild["name"], "Kithos");
    let kinds: Vec<&str> = guild["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"text"));
    assert!(kinds.contains(&"voice"));
}

#[tokio::test]
async fn a_message_round_trips_and_is_listed() {
    let (app, _) = setup().await;
    let (token, _) = register(&app, "dvrkxn", "correct horse battery").await;
    let channel = first_text_channel(&app, &token).await;

    let (status, sent) = call(
        &app,
        "POST",
        &format!("/channels/{channel}/messages"),
        Some(&token),
        Some(serde_json::json!({ "body": "  hello there  " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sent["body"], "hello there");
    assert_eq!(sent["channel_id"], channel);

    let (status, page) = call(&app, "GET", &format!("/channels/{channel}/messages"), Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let messages = page["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["body"], "hello there");
    assert_eq!(page["has_more"], false);
}

#[tokio::test]
async fn an_empty_message_is_rejected() {
    let (app, _) = setup().await;
    let (token, _) = register(&app, "dvrkxn", "correct horse battery").await;
    let channel = first_text_channel(&app, &token).await;

    let (status, body) = call(
        &app,
        "POST",
        &format!("/channels/{channel}/messages"),
        Some(&token),
        Some(serde_json::json!({ "body": "   " })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "message is empty");
}

#[tokio::test]
async fn pagination_walks_backwards_without_gaps_or_repeats() {
    let (app, _) = setup().await;
    let (token, _) = register(&app, "dvrkxn", "correct horse battery").await;
    let channel = first_text_channel(&app, &token).await;

    for n in 1..=5 {
        let (status, _) = call(
            &app,
            "POST",
            &format!("/channels/{channel}/messages"),
            Some(&token),
            Some(serde_json::json!({ "body": format!("message {n}") })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (_, newest) = call(
        &app,
        "GET",
        &format!("/channels/{channel}/messages?limit=2"),
        Some(&token),
        None,
    )
    .await;
    let newest_bodies: Vec<String> = newest["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(newest_bodies, vec!["message 4", "message 5"]);
    assert_eq!(newest["has_more"], true);

    let oldest_id = newest["messages"][0]["id"].as_i64().unwrap();
    let (_, older) = call(
        &app,
        "GET",
        &format!("/channels/{channel}/messages?limit=2&before={oldest_id}"),
        Some(&token),
        None,
    )
    .await;
    let older_bodies: Vec<String> = older["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(older_bodies, vec!["message 2", "message 3"]);
    assert_eq!(older["has_more"], true);
}

#[tokio::test]
async fn a_non_member_cannot_read_or_write_a_channel() {
    let (app, pool) = setup().await;
    let (alice, _) = register(&app, "alice", "correct horse battery").await;
    let (bob, _) = register(&app, "bob", "correct horse battery").await;

    let guild_id: i64 = sqlx::query_scalar("insert into guilds (name) values ('Other') returning id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let channel_id: i64 = sqlx::query_scalar(
        "insert into channels (guild_id, name, kind) values (?, 'secret', 'text') returning id",
    )
    .bind(guild_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (alice_status, _) = call(
        &app,
        "POST",
        &format!("/channels/{channel_id}/messages"),
        Some(&alice),
        Some(serde_json::json!({ "body": "not yours" })),
    )
    .await;
    assert_eq!(alice_status, StatusCode::FORBIDDEN);

    let (bob_status, _) = call(&app, "GET", &format!("/channels/{channel_id}/messages"), Some(&bob), None).await;
    assert_eq!(bob_status, StatusCode::FORBIDDEN);
}
