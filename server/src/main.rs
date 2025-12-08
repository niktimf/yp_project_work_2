mod client_handler;
mod generator;
mod server;

use anyhow::Result;
use server::{Server, ServerConfig};

fn main() -> Result<()> {
    let config = ServerConfig::default();
    let server = Server::new(config);
    server.run()
}
