use anyhow::Result;
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use log::{debug, info, warn};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{StockQuote, Tickers, UdpAddr};

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub target: UdpAddr,
    pub tickers: Tickers,
    pub last_ping: Instant,
}

impl ClientInfo {
    pub fn new(target: UdpAddr, tickers: Tickers) -> Self {
        Self {
            target,
            tickers,
            last_ping: Instant::now(),
        }
    }

    pub fn touch(&mut self) {
        self.last_ping = Instant::now();
    }

    #[must_use]
    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.last_ping.elapsed() > timeout
    }

    #[must_use]
    pub fn is_subscribed(&self, ticker: &str) -> bool {
        self.tickers.contains(ticker)
    }
}

#[derive(Clone)]
pub struct ClientManager {
    clients: Arc<Mutex<HashMap<UdpAddr, ClientInfo>>>,
    ping_timeout: Duration,
}

impl ClientManager {
    pub const DEFAULT_PING_TIMEOUT: Duration = Duration::from_secs(5);
    pub fn new(ping_timeout: Duration) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            ping_timeout,
        }
    }

    pub fn register(&self, target: UdpAddr, tickers: &Tickers) {
        let client = ClientInfo::new(target, tickers.clone());

        info!("Registering client {target} for tickers: {tickers}");

        self.clients.lock().insert(target, client);
    }

    pub fn update_ping(&self, target: &UdpAddr) -> bool {
        let mut clients = self.clients.lock();

        clients.get_mut(target).map_or_else(
            || {
                debug!("Ping from unknown client {target}");
                false
            },
            |client| {
                client.touch();
                debug!("Ping from {target}");
                true
            },
        )
    }

    pub fn remove(&self, target: &UdpAddr) {
        if self.clients.lock().remove(target).is_some() {
            info!("Removed client {target}");
        }
    }

    pub fn remove_expired(&self) -> Vec<UdpAddr> {
        let mut clients = self.clients.lock();
        let timeout = self.ping_timeout;
        let mut removed = Vec::new();

        clients.retain(|target, info| {
            if info.is_expired(timeout) {
                warn!("Client {target} timed out");
                removed.push(*target);
                false
            } else {
                true
            }
        });

        removed
    }

    pub fn snapshot(&self) -> Vec<ClientInfo> {
        self.clients.lock().values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.clients.lock().len()
    }

    pub fn contains(&self, target: &UdpAddr) -> bool {
        self.clients.lock().contains_key(target)
    }
}

impl Default for ClientManager {
    fn default() -> Self {
        Self::new(Self::DEFAULT_PING_TIMEOUT)
    }
}

pub struct ClientStreamer {
    addr: UdpAddr,
    tickers: Tickers,
    socket: UdpSocket,
    quote_rx: Receiver<StockQuote>,
    stop_tx: Sender<UdpAddr>,
}

impl ClientStreamer {
    const QUOTE_POLL_INTERVAL: Duration = Duration::from_millis(10);
    pub fn new(
        addr: UdpAddr,
        tickers: Tickers,
        quote_rx: Receiver<StockQuote>,
        stop_tx: Sender<UdpAddr>,
    ) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;

        Ok(Self {
            addr,
            tickers,
            socket,
            quote_rx,
            stop_tx,
        })
    }

    pub fn run(self) {
        info!("Starting stream to {} for tickers: {}", self.addr, self.tickers);

        if let Err(e) = self.stream_loop() {
            warn!("Streamer for {} stopped: {}", self.addr, e);
        }

        let _ = self.stop_tx.send(self.addr);
        info!("Stream to {} ended", self.addr);
    }

    fn stream_loop(&self) -> Result<()> {
        loop {
            match self.quote_rx.recv_timeout(Self::QUOTE_POLL_INTERVAL) {
                Ok(quote) => {
                    self.maybe_send_quote(&quote)?;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    debug!("Quote channel disconnected for {}", self.addr);
                    break;
                }
            }
        }
        Ok(())
    }

    fn maybe_send_quote(&self, quote: &StockQuote) -> Result<()> {
        if !self.tickers.contains(&quote.ticker) {
            return Ok(());
        }

        let data = quote.to_bytes();
        let addr = self.addr.socket_addr();

        self.socket.send_to(&data, addr)?;
        debug!("Sent {} to {}", quote.ticker, self.addr);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use std::thread;
    use std::time::Duration;

    #[fixture]
    fn target() -> UdpAddr {
        "127.0.0.1:8080".parse().unwrap()
    }

    #[fixture]
    fn tickers() -> Tickers {
        "AAPL,TSLA".parse().unwrap()
    }

    #[fixture]
    fn manager() -> ClientManager {
        ClientManager::default()
    }

    #[fixture]
    fn client_info(target: UdpAddr, tickers: Tickers) -> ClientInfo {
        ClientInfo::new(target, tickers)
    }

    mod client_info_tests {
        use super::*;

        #[rstest]
        fn new_is_not_expired(client_info: ClientInfo) {
            assert!(!client_info.is_expired(Duration::from_secs(5)));
        }

        #[rstest]
        fn touch_resets_timeout(mut client_info: ClientInfo) {
            thread::sleep(Duration::from_millis(50));
            client_info.touch();
            assert!(!client_info.is_expired(Duration::from_millis(100)));
        }

        #[rstest]
        #[case("AAPL", true)]
        #[case("TSLA", true)]
        #[case("GOOGL", false)]
        fn is_subscribed_checks_tickers(
            client_info: ClientInfo,
            #[case] ticker: &str,
            #[case] expected: bool,
        ) {
            assert_eq!(client_info.is_subscribed(ticker), expected);
        }
    }

    mod client_manager_tests {
        use super::*;

        #[rstest]
        fn register_and_contains(
            manager: ClientManager,
            target: UdpAddr,
            tickers: Tickers,
        ) {
            assert!(!manager.contains(&target));
            manager.register(target, &tickers);
            assert!(manager.contains(&target));
            assert_eq!(manager.count(), 1);
        }

        #[rstest]
        fn remove_client(
            manager: ClientManager,
            target: UdpAddr,
            tickers: Tickers,
        ) {
            manager.register(target, &tickers);
            manager.remove(&target);
            assert!(!manager.contains(&target));
        }

        #[rstest]
        fn update_ping_returns_false_for_unknown(manager: ClientManager) {
            let unknown: UdpAddr = "127.0.0.1:9999".parse().unwrap();
            assert!(!manager.update_ping(&unknown));
        }

        #[rstest]
        fn update_ping_returns_true_for_known(
            manager: ClientManager,
            target: UdpAddr,
            tickers: Tickers,
        ) {
            manager.register(target, &tickers);
            assert!(manager.update_ping(&target));
        }

        #[rstest]
        fn remove_expired_cleans_old_clients(
            target: UdpAddr,
            tickers: Tickers,
        ) {
            let manager = ClientManager::new(Duration::from_millis(10));
            manager.register(target, &tickers);
            thread::sleep(Duration::from_millis(50));

            let removed = manager.remove_expired();

            assert_eq!(removed.len(), 1);
            assert_eq!(removed[0], target);
            assert!(!manager.contains(&target));
        }

        #[rstest]
        fn remove_expired_keeps_active_clients(
            target: UdpAddr,
            tickers: Tickers,
        ) {
            let manager = ClientManager::new(Duration::from_secs(10));
            manager.register(target, &tickers);

            let removed = manager.remove_expired();

            assert!(removed.is_empty());
            assert!(manager.contains(&target));
        }

        #[rstest]
        #[case(8080, 8081)]
        #[case(9000, 9001)]
        fn snapshot_returns_copy(
            manager: ClientManager,
            tickers: Tickers,
            #[case] port1: u16,
            #[case] port2: u16,
        ) {
            let target1: UdpAddr =
                format!("127.0.0.1:{port1}").parse().unwrap();
            let target2: UdpAddr =
                format!("127.0.0.1:{port2}").parse().unwrap();
            manager.register(target1, &tickers);
            manager.register(target2, &tickers);

            let snapshot = manager.snapshot();
            assert_eq!(snapshot.len(), 2);
        }

        #[rstest]
        fn is_thread_safe(manager: ClientManager, tickers: Tickers) {
            let manager_clone = manager.clone();
            let tickers_clone = tickers.clone();

            let handle = thread::spawn(move || {
                let target: UdpAddr = "127.0.0.1:9000".parse().unwrap();
                manager_clone.register(target, &tickers_clone);
            });

            let target: UdpAddr = "127.0.0.1:9001".parse().unwrap();
            manager.register(target, &tickers);
            handle.join().unwrap();

            assert_eq!(manager.count(), 2);
        }
    }
}
