use anyhow::{anyhow, Result};
use clap::Parser;
use common::Tickers;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about = "Quote streaming client")]
pub struct Args {
    #[arg(short, long, default_value = "127.0.0.1:5000")]
    pub server_addr: String,

    #[arg(short = 'p', long, default_value = "5001")]
    pub ping_port: u16,

    #[arg(short = 'u', long, default_value = "34254")]
    pub udp_port: u16,

    #[arg(short = 't', long, default_value = "tickers.txt")]
    pub tickers_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_addr: SocketAddr,
    pub ping_addr: SocketAddr,
    pub udp_port: u16,
    pub tickers: Tickers,
    pub ping_interval: Duration,
}

impl ClientConfig {
    pub fn from_args(args: &Args) -> Result<Self> {
        let server_addr: SocketAddr = args.server_addr.parse()?;
        let ping_addr = SocketAddr::new(server_addr.ip(), args.ping_port);
        let tickers = read_tickers(&args.tickers_file)?;

        Ok(Self {
            server_addr,
            ping_addr,
            udp_port: args.udp_port,
            tickers,
            ping_interval: Duration::from_secs(2),
        })
    }

    pub fn udp_bind_addr(&self) -> SocketAddr {
        SocketAddr::new("0.0.0.0".parse().unwrap(), self.udp_port)
    }
}

fn read_tickers(path: &PathBuf) -> Result<Tickers> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut tickers_vec = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let ticker = line.trim();
        if !ticker.is_empty() && !ticker.starts_with('#') {
            tickers_vec.push(ticker.to_uppercase());
        }
    }

    if tickers_vec.is_empty() {
        return Err(anyhow!("No tickers found in file"));
    }

    tickers_vec.join(",").parse()
}
