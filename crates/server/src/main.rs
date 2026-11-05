use kithos_server::{app, init_db, AppState};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_target(false).init();

    let database = std::env::var("KITHOS_DB")
        .unwrap_or_else(|_| "sqlite:kithos.db?mode=rwc".to_string());
    let address = std::env::var("KITHOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let pool = init_db(&database).await?;
    let listener = TcpListener::bind(&address).await?;

    tracing::info!("listening on {address}");
    axum::serve(listener, app(AppState::new(pool))).await?;
    Ok(())
}
