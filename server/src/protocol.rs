use anyhow::{anyhow, Result};
use nonempty::NonEmpty;
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::str::FromStr;
use url::Url;

#[derive(Debug, Clone)]
struct Tickers(NonEmpty<String>);

impl Tickers {
    const TICKER_SEPARATOR: char = ',';

    pub fn one(ticker: impl Into<String>) -> Self {
        Self(NonEmpty::new(ticker.into().to_uppercase()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub fn contains(&self, ticker: &str) -> bool {
        self.0.iter().any(|t| t == ticker)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl FromStr for Tickers {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let tickers: Vec<String> = s
            .split(Self::TICKER_SEPARATOR)
            .map(|t| t.trim().to_uppercase())
            .filter(|t| !t.is_empty())
            .collect();

        NonEmpty::from_vec(tickers)
            .map(Tickers)
            .ok_or(anyhow!("No valid tickers provided"))
    }
}

impl fmt::Display for Tickers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut iter = self.0.iter();
        // Can not be empty
        write!(f, "{}", iter.next().expect("Tickers is empty"))?;

        for ticker in iter {
            let separator = Self::TICKER_SEPARATOR;
            write!(f, "{separator}{ticker}")?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpAddr(SocketAddr);

impl UdpAddr {
    const UDP_SCHEME: &str = "udp";
    fn from_url(url: &Url) -> Result<Self> {
        if url.scheme() != Self::UDP_SCHEME {
            return Err(anyhow!(
                "Expected '{}' scheme, got '{}'",
                Self::UDP_SCHEME,
                url.scheme()
            ));
        }

        let host = url.host_str().ok_or(anyhow!("Missing host in URL"))?;

        let port = url.port().ok_or(anyhow!("Missing port in URL"))?;

        Self::from_host_port(&format!("{host}:{port}"))
    }

    fn from_host_port(s: &str) -> Result<Self> {
        let addr = s
            .to_socket_addrs()?
            .next()
            .ok_or(anyhow!("Could not resolve address: {s}"))?;

        Ok(Self(addr))
    }
}

impl FromStr for UdpAddr {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        // Try URL: udp://host:port
        if let Ok(url) = Url::parse(s) {
            return Self::from_url(&url);
        }

        // Try host:port
        Self::from_host_port(s)
    }
}

impl fmt::Display for UdpAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", Self::UDP_SCHEME, self.0)
    }
}

impl From<UdpAddr> for SocketAddr {
    fn from(target: UdpAddr) -> Self {
        target.0
    }
}

impl From<SocketAddr> for UdpAddr {
    fn from(addr: SocketAddr) -> Self {
        Self(addr)
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Stream { udp_addr: UdpAddr, tickers: Tickers },
    Ping,
}

impl Command {
    pub fn stream(udp_addr: UdpAddr, tickers: Tickers) -> Self {
        Self::Stream { udp_addr, tickers }
    }
}

impl FromStr for Command {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut parts = s.trim().split_whitespace();
        let cmd = parts.next().ok_or(anyhow!("Empty command"))?;
        match cmd.to_uppercase().as_str() {
            "STREAM" => {
                let udp_addr: UdpAddr = parts
                    .next()
                    .ok_or(anyhow!("STREAM: missing UDP address"))?
                    .parse()?;

                let tickers: Tickers = parts
                    .next()
                    .ok_or(anyhow!("STREAM: missing tickers"))?
                    .parse()?;

                if parts.next().is_some() {
                    return Err(anyhow!("STREAM: too many arguments"));
                }

                Ok(Self::stream(udp_addr, tickers))
            }
            "PING" => Ok(Self::Ping),
            other => Err(anyhow!("Unknown command: {other}")),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stream { udp_addr, tickers } => {
                write!(f, "STREAM {udp_addr} {tickers}")
            }
            Self::Ping => write!(f, "PING"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Ok,
    Error(String),
}

impl FromStr for Response {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();

        if s == "OK" {
            return Ok(Self::Ok);
        }

        if let Some(msg) = s.strip_prefix("ERR ") {
            return Ok(Self::Error(msg.to_string()));
        }

        Err(anyhow!("Invalid response: {s}"))
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => write!(f, "OK"),
            Self::Error(msg) => write!(f, "ERR {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod udp_addr {
        use super::*;
        use rstest::rstest;

        #[rstest]
        fn parse_with_scheme(#[case] addr: &str) {
            let udp_addr: UdpAddr = "udp://127.0.0.1:8080".parse().unwrap();
            assert_eq!(udp_addr.0.port(), 8080);
        }

        #[test]
        fn parse_without_scheme() {
            let udp_addr: UdpAddr = "127.0.0.1:9000".parse().unwrap();
            assert_eq!(udp_addr.0.port(), 9000);
        }

        #[rstest]
        #[case("tcp://127.0.0.1:8080")]
        #[case("http://127.0.0.1:8080")]
        fn rejects_wrong_scheme(#[case] addr: &str) {
            assert!(addr.parse::<UdpAddr>().is_err());
        }

        #[test]
        fn rejects_missing_port() {
            assert!("udp://127.0.0.1".parse::<UdpAddr>().is_err());
        }

        #[test]
        fn display_roundtrip() {
            let original: UdpAddr = "udp://192.168.1.1:5000".parse().unwrap();
            let serialized = original.to_string();
            let parsed: UdpAddr = serialized.parse().unwrap();

            assert_eq!(original, parsed);
        }
    }

    mod tickers {
        use super::*;

        #[test]
        fn parse_multiple() {
            let tickers: Tickers = "AAPL, tsla , GOOGL".parse().unwrap();
            assert_eq!(tickers.len(), 3);
            assert!(tickers.contains("TSLA"));
        }

        #[test]
        fn rejects_empty() {
            assert!(",,,".parse::<Tickers>().is_err());
        }
    }

    mod command {
        use super::*;

        #[test]
        fn roundtrip() {
            let original = Command::stream(
                "udp://192.168.1.1:9000".parse().unwrap(),
                Tickers::one("GOOGL"),
            );

            let serialized = original.to_string();
            let parsed: Command = serialized.parse().unwrap();

            assert_eq!(original, parsed);
        }
    }
}
