use anyhow::{anyhow, Result};
use clap::Parser;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StockQuote {
    ticker: String,
    price: Decimal,
    volume: u32,
    timestamp: u64,
}

impl StockQuote {
    fn from_string(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('|').collect();
        if parts.len() != 4 {
            return Err(anyhow!(
                "Invalid quote format: expected 4 parts, got {}",
                parts.len()
            ));
        }

        Ok(StockQuote {
            ticker: parts[0].to_string(),
            price: parts[1].parse()?,
            volume: parts[2].parse()?,
            timestamp: parts[3].parse()?,
        })
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address and port of the TCP server (e.g., 127.0.0.1:8080)
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    server_addr: String,

    /// Port for receiving UDP data
    #[arg(short = 'u', long, default_value = "34254")]
    udp_port: u16,

    /// Path to file containing ticker symbols
    #[arg(short = 't', long, default_value = "tickers.txt")]
    tickers_file: String,
}

fn read_tickers(file_path: &str) -> Result<Vec<String>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let mut tickers = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let ticker = line.trim();
        if !ticker.is_empty() && !ticker.starts_with('#') {
            tickers.push(ticker.to_uppercase());
        }
    }

    if tickers.is_empty() {
        return Err(anyhow!("No tickers found in file"));
    }

    Ok(tickers)
}

fn send_stream_command(
    tcp_stream: &mut TcpStream,
    udp_addr: SocketAddr,
    tickers: &[String],
) -> Result<()> {
    let tickers_str = tickers.join(",");
    let command = format!("STREAM udp://{} {}\n", udp_addr, tickers_str);

    tcp_stream.write_all(command.as_bytes())?;
    tcp_stream.flush()?;

    let mut response = vec![0u8; 1024];
    let n = tcp_stream.read(&mut response)?;
    let response_str = String::from_utf8_lossy(&response[..n]);

    if response_str.trim().starts_with("OK") {
        println!("Server accepted STREAM command");
        Ok(())
    } else if response_str.trim().starts_with("ERR") {
        Err(anyhow!("Server error: {}", response_str.trim()))
    } else {
        Err(anyhow!("Unexpected server response: {}", response_str.trim()))
    }
}

fn ping_thread(
    server_addr: SocketAddr,
    udp_socket: Arc<UdpSocket>,
    running: Arc<AtomicBool>,
) {
    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        if let Err(e) = udp_socket.send_to(b"PING", server_addr) {
            eprintln!("Failed to send PING: {}", e);
        } else {
            println!("Sent PING to server");
        }

        thread::sleep(Duration::from_secs(2));
    }
}

fn receive_quotes(udp_socket: Arc<UdpSocket>, running: Arc<AtomicBool>) {
    let mut buf = [0u8; 4096];

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        match udp_socket.recv_from(&mut buf) {
            Ok((len, _addr)) => {
                let data = &buf[..len];
                let data_str = String::from_utf8_lossy(data);

                match StockQuote::from_string(&data_str) {
                    Ok(quote) => {
                        println!(
                            "[{}] {} - Price: {}, Volume: {}",
                            quote.timestamp,
                            quote.ticker,
                            quote.price,
                            quote.volume
                        );
                    }
                    Err(e) => {
                        if data_str.trim() != "PONG" {
                            eprintln!("Failed to parse quote: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                if running.load(Ordering::Relaxed) {
                    eprintln!("Failed to receive UDP data: {}", e);
                }
            }
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Reading tickers from {}", args.tickers_file);
    let tickers = read_tickers(&args.tickers_file)?;
    println!("Loaded {} tickers: {:?}", tickers.len(), tickers);

    println!("Connecting to TCP server at {}", args.server_addr);
    let mut tcp_stream = TcpStream::connect(&args.server_addr)?;
    tcp_stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let udp_addr: SocketAddr =
        format!("127.0.0.1:{}", args.udp_port).parse()?;
    println!("Setting up UDP socket on {}", udp_addr);
    let udp_socket = Arc::new(UdpSocket::bind(udp_addr)?);
    udp_socket.set_read_timeout(Some(Duration::from_secs(5)))?;

    println!("Sending STREAM command to server");
    send_stream_command(&mut tcp_stream, udp_addr, &tickers)?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        println!("\nShutting down...");
        r.store(false, Ordering::Relaxed);
    })
    .expect("Error setting Ctrl-C handler");

    let server_addr: SocketAddr = args.server_addr.parse()?;

    let udp_socket_ping = udp_socket.clone();
    let running_ping = running.clone();
    let ping_handle = thread::spawn(move || {
        ping_thread(server_addr, udp_socket_ping, running_ping);
    });

    let running_recv = running.clone();
    let recv_handle = thread::spawn(move || {
        receive_quotes(udp_socket, running_recv);
    });

    ping_handle.join().unwrap();
    recv_handle.join().unwrap();

    println!("Client shutdown complete");
    Ok(())
}

use std::io::Read;
