//! ADAM Server - Application entry point

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = "0.0.0.0:3000".parse().expect("valid address");
    tracing::info!("ADAM server starting on {}", addr);

    // TODO: Initialize application
}
