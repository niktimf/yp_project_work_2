use anyhow::Result;
use crossbeam::channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::quote::StockQuote;

pub struct ClientInfo {
    pub udp_addr: SocketAddr,
    pub tickers: Vec<String>,
    pub last_ping: Instant,
}

pub struct ClientManager {
    clients: Arc<Mutex<HashMap<SocketAddr, ClientInfo>>>,
    ping_timeout: Duration,
}

impl ClientManager {
    pub fn new(ping_timeout_secs: u64) -> Self {
        ClientManager {
            clients: Arc::new(Mutex::new(HashMap::new())),
            ping_timeout: Duration::from_secs(ping_timeout_secs),
        }
    }

    pub fn add_client(&self, udp_addr: SocketAddr, tickers: Vec<String>) {
        let mut clients = self.clients.lock();
        clients.insert(
            udp_addr,
            ClientInfo {
                udp_addr,
                tickers,
                last_ping: Instant::now(),
            },
        );
    }

    pub fn update_ping(&self, addr: &SocketAddr) {
        let mut clients = self.clients.lock();
        if let Some(client) = clients.get_mut(addr) {
            client.last_ping = Instant::now();
        }
    }

    pub fn remove_inactive_clients(&self) -> Vec<SocketAddr> {
        let mut clients = self.clients.lock();
        let now = Instant::now();
        let mut removed = Vec::new();

        clients.retain(|addr, info| {
            if now.duration_since(info.last_ping) > self.ping_timeout {
                removed.push(*addr);
                false
            } else {
                true
            }
        });

        removed
    }

    pub fn get_clients(&self) -> Vec<ClientInfo> {
        self.clients.lock().values().cloned().collect()
    }
}

impl Clone for ClientInfo {
    fn clone(&self) -> Self {
        ClientInfo {
            udp_addr: self.udp_addr,
            tickers: self.tickers.clone(),
            last_ping: self.last_ping,
        }
    }
}

pub fn handle_client_stream(
    udp_addr: SocketAddr,
    tickers: Vec<String>,
    quote_rx: Receiver<StockQuote>,
    stop_tx: Sender<SocketAddr>,
) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;

    println!("Starting stream to {} for tickers: {:?}", udp_addr, tickers);

    loop {
        // Check for quotes
        match quote_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(quote) => {
                if tickers.contains(&quote.ticker) {
                    let data = quote.to_bytes();
                    if let Err(e) = socket.send_to(&data, udp_addr) {
                        eprintln!(
                            "Failed to send quote to {}: {}",
                            udp_addr, e
                        );
                        stop_tx.send(udp_addr)?;
                        break;
                    }
                }
            }
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                println!("Quote channel disconnected for {}", udp_addr);
                break;
            }
        }
    }

    Ok(())
}
