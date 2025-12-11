mod client;
mod config;

use anyhow::Result;
use clap::Parser;
use client::Client;
use config::{Args, ClientConfig};
use log::error;
use std::sync::atomic::Ordering;

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let args = Args::parse();
    let config = ClientConfig::from_args(&args)?;
    let client = Client::new(config);

    let running = client.running();
    ctrlc::set_handler(move || {
        running.store(false, Ordering::SeqCst);
    })?;

    if let Err(e) = client.run() {
        error!("Client error: {e}");
        return Err(e);
    }

    Ok(())
}
