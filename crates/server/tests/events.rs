mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{call, first_text_channel, register, setup};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn serve(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    address.to_string()
}

#[tokio::test]
async fn a_posted_message_reaches_a_connected_listener() {
    let (router, _pool) = setup().await;
    let address = serve(router.clone()).await;

    let (token, _) = register(&router, "dvrkxn", "correct horse battery").await;
    let channel = first_text_channel(&router, &token).await;

    let mut request = format!("ws://{address}/events").into_client_request().expect("request");
    request
        .headers_mut()
        .insert("x-kithos-protocol", "1".parse().expect("header"));
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().expect("header"));

    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.expect("websocket connect");

    let (status, _) = call(
        &router,
        "POST",
        &format!("/channels/{channel}/messages"),
        Some(&token),
        Some(serde_json::json!({ "body": "live message" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("event timed out")
        .expect("stream ended")
        .expect("websocket error");

    let event: serde_json::Value = serde_json::from_str(&frame.into_text().expect("text frame")).unwrap();
    assert_eq!(event["type"], "message_new");
    assert_eq!(event["data"]["body"], "live message");
    assert_eq!(event["data"]["channel_id"], channel);

    let _ = socket.close(None).await;
}

#[tokio::test]
async fn a_websocket_without_a_token_is_refused() {
    let (router, _pool) = setup().await;
    let address = serve(router.clone()).await;

    let mut request = format!("ws://{address}/events").into_client_request().expect("request");
    request
        .headers_mut()
        .insert("x-kithos-protocol", "1".parse().expect("header"));

    let result = tokio_tungstenite::connect_async(request).await;
    assert!(result.is_err(), "an unauthenticated socket must not be accepted");
}
