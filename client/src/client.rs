use anyhow::{anyhow, Result};
use common::{Response, StockQuote};
use log::{debug, error, info, warn};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::config::ClientConfig;

pub struct Client {
    config: ClientConfig,
    running: Arc<AtomicBool>,
}

impl Client {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn run(&self) -> Result<()> {
        info!("Connecting to TCP server at {}", self.config.server_addr);
        let mut tcp_stream = TcpStream::connect(self.config.server_addr)?;
        tcp_stream.set_read_timeout(Some(Duration::from_secs(5)))?;

        info!("Setting up UDP socket on port {}", self.config.udp_port);
        let udp_socket =
            Arc::new(UdpSocket::bind(self.config.udp_bind_addr())?);
        udp_socket.set_read_timeout(Some(Duration::from_millis(500)))?;

        self.send_stream_command(&mut tcp_stream)?;

        let ping_handle = self.spawn_ping_thread(udp_socket.clone());
        let recv_handle = self.spawn_receive_thread(udp_socket);

        ping_handle
            .join()
            .map_err(|_| anyhow!("Ping thread panicked"))?;
        recv_handle
            .join()
            .map_err(|_| anyhow!("Receive thread panicked"))?;

        info!("Client shutdown complete");
        Ok(())
    }

    pub fn shutdown(&self) {
        info!("Initiating shutdown...");
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn running(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    fn send_stream_command(&self, tcp_stream: &mut TcpStream) -> Result<()> {
        let tickers_str = self.config.tickers.to_string();
        let udp_addr = self.config.udp_bind_addr();
        let command = format!(
            "STREAM udp://{}:{} {}\n",
            udp_addr.ip(),
            udp_addr.port(),
            tickers_str
        );

        info!("Sending command: {}", command.trim());
        tcp_stream.write_all(command.as_bytes())?;
        tcp_stream.flush()?;

        let mut reader = BufReader::new(tcp_stream.try_clone()?);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let response: Response = response_line.parse()?;
        match response {
            Response::Ok => {
                info!("Server accepted STREAM command");
                Ok(())
            }
            Response::Error(msg) => Err(anyhow!("Server error: {msg}")),
        }
    }

    fn spawn_ping_thread(&self, udp_socket: Arc<UdpSocket>) -> JoinHandle<()> {
        let ping_addr = self.config.ping_addr;
        let interval = self.config.ping_interval;
        let running = self.running.clone();

        thread::spawn(move || {
            Self::ping_loop(&udp_socket, ping_addr, interval, &running);
        })
    }

    fn ping_loop(
        socket: &Arc<UdpSocket>,
        ping_addr: std::net::SocketAddr,
        interval: Duration,
        running: &Arc<AtomicBool>,
    ) {
        while running.load(Ordering::SeqCst) {
            match socket.send_to(b"PING", ping_addr) {
                Ok(_) => debug!("Sent PING to {ping_addr}"),
                Err(e) => warn!("Failed to send PING: {e}"),
            }
            thread::sleep(interval);
        }
        debug!("Ping thread stopped");
    }

    fn spawn_receive_thread(
        &self,
        udp_socket: Arc<UdpSocket>,
    ) -> JoinHandle<()> {
        let running = self.running.clone();

        thread::spawn(move || {
            Self::receive_loop(&udp_socket, &running);
        })
    }

    fn receive_loop(socket: &Arc<UdpSocket>, running: &Arc<AtomicBool>) {
        let mut buf = [0_u8; 4096];

        while running.load(Ordering::SeqCst) {
            match socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    let data = String::from_utf8_lossy(&buf[..len]);
                    Self::handle_received_data(&data);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    if running.load(Ordering::SeqCst) {
                        error!("Failed to receive UDP data: {e}");
                    }
                }
            }
        }
        debug!("Receive thread stopped");
    }

    fn handle_received_data(data: &str) {
        let data = data.trim();
        if data == "PONG" {
            debug!("Received PONG");
            return;
        }

        match data.parse::<StockQuote>() {
            Ok(quote) => {
                info!(
                    "[{}] {} - Price: {}, Volume: {}",
                    quote.timestamp, quote.ticker, quote.price, quote.volume
                );
            }
            Err(e) => warn!("Failed to parse quote '{data}': {e}"),
        }
    }
}
