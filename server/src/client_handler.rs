use anyhow::Result;
use crossbeam::channel::{Receiver, RecvTimeoutError, TryRecvError};
use log::{debug, info, warn};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{StockQuote, Tickers, UdpAddr};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClientInfo {
    pub target: UdpAddr,
    pub tickers: Tickers,
    pub last_ping: Instant,
    pub source_ip: IpAddr,
}

impl ClientInfo {
    pub fn new(target: UdpAddr, tickers: Tickers, source_ip: IpAddr) -> Self {
        Self {
            target,
            tickers,
            last_ping: Instant::now(),
            source_ip,
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
    #[allow(dead_code)]
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

    pub fn register(
        &self,
        target: UdpAddr,
        tickers: &Tickers,
        source_ip: IpAddr,
    ) {
        let client = ClientInfo::new(target, tickers.clone(), source_ip);

        info!("Registering client {target} for tickers: {tickers} (source IP: {source_ip})");

        self.clients.lock().insert(target, client);
    }

    #[allow(dead_code)]
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

    pub fn update_ping_by_source(&self, source_addr: &SocketAddr) -> bool {
        let found = self
            .clients
            .lock()
            .values_mut()
            .find(|client| client.source_ip == source_addr.ip())
            .map(|client| {
                client.touch();
            })
            .is_some();

        if !found {
            debug!("Ping from unknown source {source_addr}");
        }
        found
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn snapshot(&self) -> Vec<ClientInfo> {
        self.clients.lock().values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.clients.lock().len()
    }

    #[allow(dead_code)]
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
    stop_rx: Receiver<()>,
}

impl ClientStreamer {
    const QUOTE_POLL_INTERVAL: Duration = Duration::from_millis(10);

    pub fn new(
        addr: UdpAddr,
        tickers: Tickers,
        quote_rx: Receiver<StockQuote>,
        stop_rx: Receiver<()>,
    ) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;

        Ok(Self {
            addr,
            tickers,
            socket,
            quote_rx,
            stop_rx,
        })
    }

    pub fn run(self) {
        info!("Starting stream to {} for tickers: {}", self.addr, self.tickers);

        if let Err(e) = self.stream_loop() {
            warn!("Streamer for {} stopped: {}", self.addr, e);
        }

        info!("Stream to {} ended", self.addr);
    }

    fn stream_loop(&self) -> Result<()> {
        loop {
            match self.stop_rx.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => {
                    debug!("Stop signal received for {}", self.addr);
                    break;
                }
                Err(TryRecvError::Empty) => {}
            }

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
    fn source_ip() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    #[fixture]
    fn client_info(
        target: UdpAddr,
        tickers: Tickers,
        source_ip: IpAddr,
    ) -> ClientInfo {
        ClientInfo::new(target, tickers, source_ip)
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
            source_ip: IpAddr,
        ) {
            assert!(!manager.contains(&target));
            manager.register(target, &tickers, source_ip);
            assert!(manager.contains(&target));
            assert_eq!(manager.count(), 1);
        }

        #[rstest]
        fn remove_client(
            manager: ClientManager,
            target: UdpAddr,
            tickers: Tickers,
            source_ip: IpAddr,
        ) {
            manager.register(target, &tickers, source_ip);
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
            source_ip: IpAddr,
        ) {
            manager.register(target, &tickers, source_ip);
            assert!(manager.update_ping(&target));
        }

        #[rstest]
        fn remove_expired_cleans_old_clients(
            target: UdpAddr,
            tickers: Tickers,
            source_ip: IpAddr,
        ) {
            let manager = ClientManager::new(Duration::from_millis(10));
            manager.register(target, &tickers, source_ip);
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
            source_ip: IpAddr,
        ) {
            let manager = ClientManager::new(Duration::from_secs(10));
            manager.register(target, &tickers, source_ip);

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
            source_ip: IpAddr,
            #[case] port1: u16,
            #[case] port2: u16,
        ) {
            let target1: UdpAddr =
                format!("127.0.0.1:{port1}").parse().unwrap();
            let target2: UdpAddr =
                format!("127.0.0.1:{port2}").parse().unwrap();
            manager.register(target1, &tickers, source_ip);
            manager.register(target2, &tickers, source_ip);

            let snapshot = manager.snapshot();
            assert_eq!(snapshot.len(), 2);
        }

        #[rstest]
        fn is_thread_safe(
            manager: ClientManager,
            tickers: Tickers,
            source_ip: IpAddr,
        ) {
            let manager_clone = manager.clone();
            let tickers_clone = tickers.clone();

            let handle = thread::spawn(move || {
                let target: UdpAddr = "127.0.0.1:9000".parse().unwrap();
                manager_clone.register(target, &tickers_clone, source_ip);
            });

            let target: UdpAddr = "127.0.0.1:9001".parse().unwrap();
            manager.register(target, &tickers, source_ip);
            handle.join().unwrap();

            assert_eq!(manager.count(), 2);
        }

        #[rstest]
        fn update_ping_by_source_works(
            manager: ClientManager,
            target: UdpAddr,
            tickers: Tickers,
            source_ip: IpAddr,
        ) {
            manager.register(target, &tickers, source_ip);
            let source_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
            assert!(manager.update_ping_by_source(&source_addr));
        }
    }
}
