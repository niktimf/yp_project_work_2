mod client_handler;
mod generator;
mod protocol;
mod quote;

use anyhow::Result;
use crossbeam::channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::client_handler::{handle_client_stream, ClientManager};
use crate::generator::QuoteGenerator;
use crate::protocol::Command;
use crate::quote::StockQuote;

const TCP_PORT: u16 = 5000;
const UDP_PING_PORT: u16 = 5001;
const PING_TIMEOUT_SECS: u64 = 5;
const QUOTE_INTERVAL_MS: u64 = 100;

fn main() -> Result<()> {
    println!("Starting Quote Server...");

    // Create channels for quote distribution
    let (quote_tx, _) = unbounded::<StockQuote>();
    let client_channels: Arc<Mutex<HashMap<SocketAddr, Sender<StockQuote>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Create client manager
    let client_manager = Arc::new(ClientManager::new(PING_TIMEOUT_SECS));

    // Start quote generator thread
    let generator_tx = quote_tx.clone();
    let generator_channels = client_channels.clone();
    thread::spawn(move || {
        if let Err(e) = run_quote_generator(generator_tx, generator_channels) {
            eprintln!("Quote generator error: {}", e);
        }
    });

    // Start ping listener thread
    let ping_manager = client_manager.clone();
    thread::spawn(move || {
        if let Err(e) = run_ping_listener(ping_manager) {
            eprintln!("Ping listener error: {}", e);
        }
    });

    // Start inactive client cleanup thread
    let cleanup_manager = client_manager.clone();
    let cleanup_channels = client_channels.clone();
    thread::spawn(move || {
        run_cleanup_thread(cleanup_manager, cleanup_channels);
    });

    // Start TCP server
    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{}", TCP_PORT))?;
    println!("TCP server listening on port {}", TCP_PORT);

    for stream in tcp_listener.incoming() {
        match stream {
            Ok(stream) => {
                let manager = client_manager.clone();
                let channels = client_channels.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_tcp_client(stream, manager, channels)
                    {
                        eprintln!("Client handler error: {}", e);
                    }
                });
            }
            Err(e) => eprintln!("Failed to accept connection: {}", e),
        }
    }

    Ok(())
}

fn run_quote_generator(
    _base_tx: Sender<StockQuote>,
    client_channels: Arc<Mutex<HashMap<SocketAddr, Sender<StockQuote>>>>,
) -> Result<()> {
    let mut generator = QuoteGenerator::new();
    let all_tickers = vec![
        "AAPL", "GOOGL", "TSLA", "MSFT", "AMZN", "NVDA", "META", "JPM", "JNJ",
        "V",
    ];

    loop {
        thread::sleep(Duration::from_millis(QUOTE_INTERVAL_MS));

        // Generate quotes for all tickers
        for ticker in &all_tickers {
            if let Ok(quote) = generator.generate_quote(ticker) {
                // Send to all clients
                let channels = client_channels.lock();
                for (_, sender) in channels.iter() {
                    let _ = sender.send(quote.clone());
                }
            }
        }
    }
}

fn run_ping_listener(client_manager: Arc<ClientManager>) -> Result<()> {
    let socket = UdpSocket::bind(format!("0.0.0.0:{}", UDP_PING_PORT))?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    println!("UDP ping listener on port {}", UDP_PING_PORT);

    let mut buf = [0u8; 1024];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, addr)) => {
                let msg = String::from_utf8_lossy(&buf[..len]);
                if msg.trim().to_uppercase() == "PING" {
                    client_manager.update_ping(&addr);
                    socket.send_to(b"PONG", addr)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => eprintln!("Ping listener error: {}", e),
        }
    }
}

fn run_cleanup_thread(
    client_manager: Arc<ClientManager>,
    client_channels: Arc<Mutex<HashMap<SocketAddr, Sender<StockQuote>>>>,
) {
    loop {
        thread::sleep(Duration::from_secs(1));

        let removed = client_manager.remove_inactive_clients();
        if !removed.is_empty() {
            let mut channels = client_channels.lock();
            for addr in removed {
                channels.remove(&addr);
                println!("Removed inactive client: {}", addr);
            }
        }
    }
}

fn handle_tcp_client(
    stream: TcpStream,
    client_manager: Arc<ClientManager>,
    client_channels: Arc<Mutex<HashMap<SocketAddr, Sender<StockQuote>>>>,
) -> Result<()> {
    let peer_addr = stream.peer_addr()?;
    println!("New TCP connection from: {}", peer_addr);

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        match Command::parse(&line) {
            Ok(Command::Stream { udp_addr, tickers }) => {
                println!(
                    "Starting stream to {} for tickers: {:?}",
                    udp_addr, tickers
                );

                // Add client to manager
                client_manager.add_client(udp_addr, tickers.clone());

                // Create channel for this client
                let (tx, rx) = unbounded();
                client_channels.lock().insert(udp_addr, tx.clone());

                // Start streaming thread
                let (stop_tx, stop_rx) = unbounded();
                thread::spawn(move || {
                    if let Err(e) =
                        handle_client_stream(udp_addr, tickers, rx, stop_tx)
                    {
                        eprintln!("Stream handler error: {}", e);
                    }
                });
            }
            Ok(Command::Ping) => {
                // Ping is handled via UDP
            }
            Err(e) => {
                eprintln!("Command parse error: {}", e);
            }
        }
    }

    println!("TCP connection closed: {}", peer_addr);
    Ok(())
}
