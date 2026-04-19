//! Binary entry point — thin shell around [`foundation_share_bridge::run`].
//!
//! Responsibilities kept here:
//! - install the tracing subscriber (logging is a binary concern, not a lib one),
//! - delegate everything else to the library.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "foundation_share_bridge=info,tower_http=info".into()),
        )
        .init();

    foundation_share_bridge::run().await
}
