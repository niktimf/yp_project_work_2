use anyhow::Result;
use crossbeam::channel::{unbounded, Sender};
use log::{error, info, warn};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::client_handler::{ClientManager, ClientStreamer};
use crate::generator::QuoteGenerator;
use common::{Command, Response, StockQuote};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub tcp_port: u16,
    pub udp_ping_port: u16,
    pub ping_timeout: Duration,
    pub quote_interval: Duration,
    pub cleanup_interval: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            tcp_port: 5000,
            udp_ping_port: 5001,
            ping_timeout: Duration::from_secs(5),
            quote_interval: Duration::from_millis(100),
            cleanup_interval: Duration::from_secs(1),
        }
    }
}

type ClientChannels = Arc<Mutex<HashMap<SocketAddr, Sender<StockQuote>>>>;

pub struct Server {
    config: ServerConfig,
    client_manager: Arc<ClientManager>,
    client_channels: ClientChannels,
}

impl Server {
    const ALL_TICKERS: [&str; 10] = [
        "AAPL", "GOOGL", "TSLA", "MSFT", "AMZN", "NVDA", "META", "JPM", "JNJ",
        "V",
    ];
    pub fn new(config: ServerConfig) -> Self {
        let client_manager = Arc::new(ClientManager::new(config.ping_timeout));
        let client_channels = Arc::new(Mutex::new(HashMap::new()));

        Self {
            config,
            client_manager,
            client_channels,
        }
    }

    pub fn run(&self) -> Result<()> {
        info!("Starting Quote Server...");

        self.spawn_quote_generator();
        self.spawn_ping_listener();
        self.spawn_cleanup_thread();
        self.run_tcp_server()
    }

    fn spawn_quote_generator(&self) {
        let channels = self.client_channels.clone();
        let interval = self.config.quote_interval;

        thread::spawn(move || {
            if let Err(e) = Self::quote_generator_loop(&channels, interval) {
                error!("Quote generator error: {e}");
            }
        });
    }

    fn quote_generator_loop(
        client_channels: &ClientChannels,
        interval: Duration,
    ) -> Result<()> {
        let mut generator = QuoteGenerator::new();

        loop {
            thread::sleep(interval);

            for ticker in &Self::ALL_TICKERS {
                if let Ok(quote) = generator.generate(ticker) {
                    let channels = client_channels.lock();
                    for sender in channels.values() {
                        let _ = sender.send(quote.clone());
                    }
                }
            }
        }
    }

    fn spawn_ping_listener(&self) {
        let manager = self.client_manager.clone();
        let port = self.config.udp_ping_port;

        thread::spawn(move || {
            if let Err(e) = Self::ping_listener_loop(&manager, port) {
                error!("Ping listener error: {e}");
            }
        });
    }

    fn ping_listener_loop(
        client_manager: &Arc<ClientManager>,
        port: u16,
    ) -> Result<()> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{port}"))?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        info!("UDP ping listener on port {port}");

        let mut buf = [0_u8; 1024];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((len, addr)) => {
                    let msg = String::from_utf8_lossy(&buf[..len]);
                    if msg.trim().eq_ignore_ascii_case("PING") {
                        client_manager.update_ping(&addr.into());
                        let _ = socket.send_to(b"PONG", addr);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => warn!("Ping listener error: {e}"),
            }
        }
    }

    fn spawn_cleanup_thread(&self) {
        let manager = self.client_manager.clone();
        let channels = self.client_channels.clone();
        let interval = self.config.cleanup_interval;

        thread::spawn(move || {
            Self::cleanup_loop(&manager, &channels, interval);
        });
    }

    fn cleanup_loop(
        client_manager: &Arc<ClientManager>,
        client_channels: &ClientChannels,
        interval: Duration,
    ) {
        loop {
            thread::sleep(interval);

            let removed = client_manager.remove_expired();
            if !removed.is_empty() {
                {
                    let mut channels = client_channels.lock();
                    for addr in &removed {
                        channels.remove(&addr.socket_addr());
                    }
                }
                for addr in &removed {
                    info!("Removed inactive client: {addr}");
                }
            }
        }
    }

    fn run_tcp_server(&self) -> Result<()> {
        let listener =
            TcpListener::bind(format!("0.0.0.0:{}", self.config.tcp_port))?;
        info!("TCP server listening on port {}", self.config.tcp_port);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let manager = self.client_manager.clone();
                    let channels = self.client_channels.clone();
                    thread::spawn(move || {
                        if let Err(e) =
                            Self::handle_tcp_client(stream, &manager, &channels)
                        {
                            error!("Client handler error: {e}");
                        }
                    });
                }
                Err(e) => error!("Failed to accept connection: {e}"),
            }
        }

        Ok(())
    }

    fn handle_tcp_client(
        mut stream: TcpStream,
        client_manager: &Arc<ClientManager>,
        client_channels: &ClientChannels,
    ) -> Result<()> {
        let peer_addr = stream.peer_addr()?;
        info!("New TCP connection from: {peer_addr}");

        let reader = BufReader::new(stream.try_clone()?);
        for line in reader.lines() {
            let line = line?;
            let response = match line.parse::<Command>() {
                Ok(Command::Stream { udp_addr, tickers }) => {
                    info!(
                        "Starting stream to {udp_addr} for tickers: {tickers}"
                    );

                    client_manager.register(udp_addr, &tickers.clone());

                    let (tx, rx) = unbounded();
                    client_channels.lock().insert(udp_addr.socket_addr(), tx);

                    let (stop_tx, _stop_rx) = unbounded();
                    thread::spawn(move || {
                        match ClientStreamer::new(
                            udp_addr, tickers, rx, stop_tx,
                        ) {
                            Ok(streamer) => streamer.run(),
                            Err(e) => error!("Stream handler error: {e}"),
                        }
                    });

                    Response::Ok
                }
                Ok(Command::Ping) => Response::Ok,
                Err(e) => {
                    warn!("Command parse error: {e}");
                    Response::Error(e.to_string())
                }
            };

            writeln!(stream, "{response}")?;
        }

        info!("TCP connection closed: {peer_addr}");
        Ok(())
    }
}
