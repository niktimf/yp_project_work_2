mod client_handler;
mod generator;
mod server;

use anyhow::Result;
use log::info;
use server::{Server, ServerConfig};
use std::sync::atomic::Ordering;

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let config = ServerConfig::default();
    let server = Server::new(config);

    let running = server.running();
    ctrlc::set_handler(move || {
        info!("Received Ctrl+C, shutting down...");
        running.store(false, Ordering::SeqCst);
    })?;

    server.run()
}
