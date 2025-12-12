use anyhow::Result;
use crossbeam::channel::{unbounded, Sender};
use log::{debug, error, info, warn};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::client_handler::{ClientManager, ClientStreamer};
use crate::generator::QuoteGenerator;
use common::{Command, Response, StockQuote, UdpAddr};

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

const PING_BUFFER_SIZE: usize = 1024;
const UDP_READ_TIMEOUT_SECS: u64 = 1;

type ClientChannels = Arc<Mutex<HashMap<SocketAddr, Sender<StockQuote>>>>;
type StopChannels = Arc<Mutex<HashMap<SocketAddr, Sender<()>>>>;

pub struct Server {
    config: ServerConfig,
    client_manager: Arc<ClientManager>,
    client_channels: ClientChannels,
    stop_channels: StopChannels,
    running: Arc<AtomicBool>,
}

impl Server {
    const ALL_TICKERS: [&str; 10] = [
        "AAPL", "GOOGL", "TSLA", "MSFT", "AMZN", "NVDA", "META", "JPM", "JNJ",
        "V",
    ];
    pub fn new(config: ServerConfig) -> Self {
        let client_manager = Arc::new(ClientManager::new(config.ping_timeout));
        let client_channels = Arc::new(Mutex::new(HashMap::new()));
        let stop_channels = Arc::new(Mutex::new(HashMap::new()));
        let running = Arc::new(AtomicBool::new(true));

        Self {
            config,
            client_manager,
            client_channels,
            stop_channels,
            running,
        }
    }

    pub fn running(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    pub fn run(&self) -> Result<()> {
        info!("Starting Quote Server...");

        self.spawn_quote_generator();
        self.spawn_ping_listener();
        self.spawn_cleanup_thread();
        self.run_tcp_server()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn spawn_quote_generator(&self) {
        let channels = self.client_channels.clone();
        let interval = self.config.quote_interval;
        let running = self.running.clone();

        thread::spawn(move || {
            Self::quote_generator_loop(&channels, interval, &running);
        });
    }

    fn quote_generator_loop(
        client_channels: &ClientChannels,
        interval: Duration,
        running: &Arc<AtomicBool>,
    ) {
        let mut generator = QuoteGenerator::new();

        while running.load(Ordering::SeqCst) {
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
        info!("Quote generator stopped");
    }

    fn spawn_ping_listener(&self) {
        let manager = self.client_manager.clone();
        let port = self.config.udp_ping_port;
        let running = self.running.clone();

        thread::spawn(move || {
            if let Err(e) = Self::ping_listener_loop(&manager, port, &running) {
                error!("Ping listener error: {e}");
            }
        });
    }

    fn ping_listener_loop(
        client_manager: &Arc<ClientManager>,
        port: u16,
        running: &Arc<AtomicBool>,
    ) -> Result<()> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{port}"))?;
        socket.set_read_timeout(Some(Duration::from_secs(
            UDP_READ_TIMEOUT_SECS,
        )))?;
        info!("UDP ping listener on port {port}");

        let mut buf = [0_u8; PING_BUFFER_SIZE];
        while running.load(Ordering::SeqCst) {
            match socket.recv_from(&mut buf) {
                Ok((len, addr)) => {
                    let msg = String::from_utf8_lossy(&buf[..len]);
                    if msg.trim().eq_ignore_ascii_case("PING") {
                        if client_manager.update_ping_by_source(&addr) {
                            debug!("Ping from {addr}");
                        }
                        let _ = socket.send_to(b"PONG", addr);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => warn!("Ping listener error: {e}"),
            }
        }
        info!("Ping listener stopped");
        Ok(())
    }

    fn spawn_cleanup_thread(&self) {
        let manager = self.client_manager.clone();
        let channels = self.client_channels.clone();
        let stop_channels = self.stop_channels.clone();
        let interval = self.config.cleanup_interval;
        let running = self.running.clone();

        thread::spawn(move || {
            Self::cleanup_loop(
                &manager,
                &channels,
                &stop_channels,
                interval,
                &running,
            );
        });
    }

    fn cleanup_loop(
        client_manager: &Arc<ClientManager>,
        client_channels: &ClientChannels,
        stop_channels: &StopChannels,
        interval: Duration,
        running: &Arc<AtomicBool>,
    ) {
        while running.load(Ordering::SeqCst) {
            thread::sleep(interval);

            let removed = client_manager.remove_expired();
            if !removed.is_empty() {
                for addr in &removed {
                    client_channels.lock().remove(&addr.socket_addr());
                    let stop_tx =
                        stop_channels.lock().remove(&addr.socket_addr());
                    if let Some(tx) = stop_tx {
                        let _ = tx.send(());
                    }
                    info!("Removed inactive client: {addr}");
                }
            }
        }
        info!("Cleanup thread stopped");
    }

    fn run_tcp_server(&self) -> Result<()> {
        let listener =
            TcpListener::bind(format!("0.0.0.0:{}", self.config.tcp_port))?;
        listener.set_nonblocking(true)?;
        info!("TCP server listening on port {}", self.config.tcp_port);

        while self.is_running() {
            match listener.accept() {
                Ok((stream, _)) => {
                    let manager = self.client_manager.clone();
                    let channels = self.client_channels.clone();
                    let stop_channels = self.stop_channels.clone();
                    thread::spawn(move || {
                        if let Err(e) = Self::handle_tcp_client(
                            stream,
                            &manager,
                            &channels,
                            &stop_channels,
                        ) {
                            error!("Client handler error: {e}");
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => error!("Failed to accept connection: {e}"),
            }
        }

        info!("TCP server stopped");
        Ok(())
    }

    fn handle_tcp_client(
        mut stream: TcpStream,
        client_manager: &Arc<ClientManager>,
        client_channels: &ClientChannels,
        stop_channels: &StopChannels,
    ) -> Result<()> {
        let peer_addr = stream.peer_addr()?;
        info!("New TCP connection from: {peer_addr}");

        let reader = BufReader::new(stream.try_clone()?);
        for line in reader.lines() {
            let line = line?;
            let response = match line.parse::<Command>() {
                Ok(Command::Stream { udp_addr, tickers }) => {
                    Self::handle_stream_command(
                        udp_addr,
                        tickers,
                        peer_addr,
                        client_manager,
                        client_channels,
                        stop_channels,
                    )
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

    fn handle_stream_command(
        udp_addr: UdpAddr,
        tickers: common::Tickers,
        peer_addr: SocketAddr,
        client_manager: &Arc<ClientManager>,
        client_channels: &ClientChannels,
        stop_channels: &StopChannels,
    ) -> Response {
        info!("Starting stream to {udp_addr} for tickers: {tickers}");

        client_manager.register(udp_addr, &tickers, peer_addr.ip());

        let (tx, rx) = unbounded();
        let (stop_tx, stop_rx) = unbounded();

        client_channels.lock().insert(udp_addr.socket_addr(), tx);
        stop_channels.lock().insert(udp_addr.socket_addr(), stop_tx);

        thread::spawn(move || {
            match ClientStreamer::new(udp_addr, tickers, rx, stop_rx) {
                Ok(streamer) => streamer.run(),
                Err(e) => error!("Stream handler error: {e}"),
            }
        });

        Response::Ok
    }
}
