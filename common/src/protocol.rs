use anyhow::{anyhow, Result};
use nonempty::NonEmpty;
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::str::FromStr;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tickers(NonEmpty<String>);

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

    pub fn is_empty(&self) -> bool {
        false
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
        write!(f, "{}", iter.next().expect("Tickers is empty"))?;

        for ticker in iter {
            let separator = Self::TICKER_SEPARATOR;
            write!(f, "{separator}{ticker}")?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UdpAddr(SocketAddr);

impl UdpAddr {
    const UDP_SCHEME: &str = "udp";

    pub fn socket_addr(&self) -> SocketAddr {
        self.0
    }

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
        if let Ok(url) = Url::parse(s) {
            return Self::from_url(&url);
        }

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

#[derive(Debug, Clone, PartialEq, Eq)]
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
        let mut parts = s.split_whitespace();
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
        let s = s.trim_start();

        if s == "OK" || s.trim_end() == "OK" {
            return Ok(Self::Ok);
        }

        if let Some(msg) = s.strip_prefix("ERR ") {
            return Ok(Self::Error(msg.to_string()));
        }

        if s == "ERR" || s.trim_end() == "ERR" {
            return Ok(Self::Error(String::new()));
        }

        Err(anyhow!("Invalid response: {s}"))
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => write!(f, "OK"),
            Self::Error(msg) if msg.is_empty() => write!(f, "ERR"),
            Self::Error(msg) => write!(f, "ERR {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::{any, prop, Just, Strategy};
    use proptest::{prop_assert, prop_assert_eq, prop_oneof, proptest};
    use rstest::rstest;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

    fn valid_ipv4_addr() -> impl Strategy<Value = SocketAddr> {
        (any::<[u8; 4]>(), 1u16..=u16::MAX).prop_map(|(octets, port)| {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(octets), port))
        })
    }

    fn valid_ipv6_addr() -> impl Strategy<Value = SocketAddr> {
        (any::<[u8; 16]>(), 1u16..=u16::MAX).prop_map(|(octets, port)| {
            SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(octets),
                port,
                0,
                0,
            ))
        })
    }

    fn valid_socket_addr() -> impl Strategy<Value = SocketAddr> {
        prop_oneof![valid_ipv4_addr(), valid_ipv6_addr()]
    }

    fn valid_udp_target() -> impl Strategy<Value = UdpAddr> {
        valid_socket_addr().prop_map(UdpAddr::from)
    }

    fn valid_ticker() -> impl Strategy<Value = String> {
        "[A-Z]{1,10}"
    }

    fn valid_tickers() -> impl Strategy<Value = Tickers> {
        prop::collection::vec(valid_ticker(), 1..10)
            .prop_map(|v| Tickers::from_str(&v.join(",")).unwrap())
    }

    fn valid_stream_command() -> impl Strategy<Value = Command> {
        (valid_udp_target(), valid_tickers())
            .prop_map(|(target, tickers)| Command::stream(target, tickers))
    }

    fn valid_ping_command() -> impl Strategy<Value = Command> {
        Just(Command::Ping)
    }

    fn valid_command() -> impl Strategy<Value = Command> {
        prop_oneof![valid_stream_command(), valid_ping_command(),]
    }

    fn valid_response() -> impl Strategy<Value = Response> {
        prop_oneof![
            Just(Response::Ok),
            Just(Response::Error(String::new())),
            "[a-zA-Z0-9]{1,50}".prop_map(Response::Error),
        ]
    }

    mod udp_addr {
        use super::*;

        proptest! {
            #[test]
            fn roundtrip(addr in valid_socket_addr()) {
                let target = UdpAddr::from(addr);
                let serialized = target.to_string();
                let parsed: UdpAddr = serialized.parse().unwrap();
                prop_assert_eq!(target, parsed);
            }

            #[test]
            fn parses_with_scheme(addr in valid_socket_addr()) {
                let input = format!("udp://{addr}");
                let target: UdpAddr = input.parse().unwrap();
                prop_assert_eq!(target.socket_addr(), addr);
            }

            #[test]
            fn parses_without_scheme(addr in valid_ipv4_addr()) {
                let input = addr.to_string();
                let target: UdpAddr = input.parse().unwrap();
                prop_assert_eq!(target.socket_addr(), addr);
            }

            #[test]
            fn rejects_wrong_scheme(
                scheme in "(tcp|http|https|ftp|ws)",
                addr in valid_ipv4_addr()
            ) {
                let input = format!("{scheme}://{addr}");
                prop_assert!(input.parse::<UdpAddr>().is_err());
            }

            #[test]
            fn into_socket_addr(addr in valid_socket_addr()) {
                let target = UdpAddr::from(addr);
                let converted: SocketAddr = target.into();
                prop_assert_eq!(converted, addr);
            }
        }

        #[rstest]
        #[case("udp://127.0.0.1")]
        #[case("udp://localhost")]
        fn rejects_missing_port(#[case] input: &str) {
            assert!(input.parse::<UdpAddr>().is_err());
        }

        #[rstest]
        #[case("udp://not a host:8080")]
        #[case("udp://:8080")]
        fn rejects_invalid_host(#[case] input: &str) {
            assert!(input.parse::<UdpAddr>().is_err());
        }

        #[test]
        fn parses_ipv6_with_brackets() {
            let target: UdpAddr = "udp://[::1]:8080".parse().unwrap();
            assert!(target.socket_addr().is_ipv6());
            assert_eq!(target.socket_addr().port(), 8080);
        }
    }

    mod tickers {
        use super::*;

        proptest! {
            #[test]
            fn roundtrip(tickers in valid_tickers()) {
                let serialized = tickers.to_string();
                let parsed: Tickers = serialized.parse().unwrap();
                prop_assert_eq!(tickers, parsed);
            }

            #[test]
            fn len_always_positive(tickers in valid_tickers()) {
                prop_assert!(!tickers.is_empty());
            }

            #[test]
            fn contains_all_parsed(input in prop::collection::vec(valid_ticker(), 1..5)) {
                let joined = input.join(",");
                let tickers: Tickers = joined.parse().unwrap();

                for ticker in &input {
                    prop_assert!(tickers.contains(ticker));
                }
            }

            #[test]
            fn normalizes_to_uppercase(lower in "[a-z]{1,5}") {
                let tickers: Tickers = lower.parse().unwrap();
                let upper = lower.to_uppercase();
                prop_assert!(tickers.contains(&upper));
            }

            #[test]
            fn trims_whitespace(ticker in valid_ticker()) {
                let input = format!("  {ticker}  ,  {ticker}  ");
                let tickers: Tickers = input.parse().unwrap();
                prop_assert!(tickers.contains(&ticker));
            }

            #[test]
            fn display_no_spaces(tickers in valid_tickers()) {
                let serialized = tickers.to_string();
                prop_assert!(!serialized.contains(' '));
            }
        }

        #[test]
        fn one_creates_single() {
            let tickers = Tickers::one("AAPL");
            assert_eq!(tickers.len(), 1);
            assert!(tickers.contains("AAPL"));
        }

        #[rstest]
        #[case("")]
        #[case("   ")]
        #[case(",,,")]
        #[case(" , , , ")]
        fn rejects_empty_input(#[case] input: &str) {
            assert!(input.parse::<Tickers>().is_err());
        }

        #[test]
        fn filters_empty_segments() {
            let tickers: Tickers = "AAPL,,TSLA,,,META".parse().unwrap();
            assert_eq!(tickers.len(), 3);
        }

        #[test]
        fn iter_yields_all() {
            let tickers: Tickers = "A,B,C".parse().unwrap();
            let collected: Vec<_> = tickers.iter().collect();
            assert_eq!(collected, vec!["A", "B", "C"]);
        }
    }

    mod command {
        use super::*;

        proptest! {
            #[test]
            fn roundtrip(cmd in valid_command()) {
                let serialized = cmd.to_string();
                let parsed: Command = serialized.parse().unwrap();
                prop_assert_eq!(cmd, parsed);
            }

            #[test]
            fn stream_display_format(target in valid_udp_target(), tickers in valid_tickers()) {
                let cmd = Command::stream(target, tickers);
                let serialized = cmd.to_string();

                prop_assert!(serialized.starts_with("STREAM "));
                prop_assert!(serialized.contains("udp://"));
            }

            #[test]
            fn case_insensitive_command(cmd in valid_command()) {
                let upper = cmd.to_string();
                let lower = upper.to_lowercase();
                let parsed: Command = lower.parse().unwrap();
                prop_assert_eq!(cmd, parsed);
            }
        }

        #[test]
        fn ping_display() {
            assert_eq!(Command::Ping.to_string(), "PING");
        }

        #[rstest]
        #[case("")]
        #[case("   ")]
        fn rejects_empty(#[case] input: &str) {
            assert!(input.parse::<Command>().is_err());
        }

        #[rstest]
        #[case("SUBSCRIBE udp://127.0.0.1:8080 AAPL")]
        #[case("STOP")]
        fn rejects_unknown_command(#[case] input: &str) {
            assert!(input.parse::<Command>().is_err());
        }

        #[rstest]
        #[case("STREAM")]
        #[case("STREAM udp://127.0.0.1:8080")]
        fn rejects_stream_missing_args(#[case] input: &str) {
            assert!(input.parse::<Command>().is_err());
        }

        #[rstest]
        #[case("STREAM udp://127.0.0.1:8080 AAPL extra")]
        fn rejects_stream_extra_args(#[case] input: &str) {
            assert!(input.parse::<Command>().is_err());
        }
    }

    mod response {
        use super::*;

        proptest! {
            #[test]
            fn roundtrip(resp in valid_response()) {
                let serialized = resp.to_string();
                let parsed: Response = serialized.parse().unwrap();
                prop_assert_eq!(resp, parsed);
            }

            #[test]
            fn error_preserves_message(msg in "[a-zA-Z0-9 ]{1,50}") {
                let resp = Response::Error(msg.clone());
                let serialized = resp.to_string();
                let parsed: Response = serialized.parse().unwrap();

                match parsed {
                    Response::Error(parsed_msg) => prop_assert_eq!(parsed_msg, msg),
                    _ => prop_assert!(false, "Expected Error variant"),
                }
            }
        }

        #[test]
        fn ok_display() {
            assert_eq!(Response::Ok.to_string(), "OK");
        }

        #[test]
        fn error_display() {
            let resp = Response::Error("Something failed".to_string());
            assert_eq!(resp.to_string(), "ERR Something failed");
        }

        #[rstest]
        #[case("")]
        #[case("OKAY")]
        #[case("ERROR message")]
        fn rejects_invalid(#[case] input: &str) {
            assert!(input.parse::<Response>().is_err());
        }

        #[test]
        fn parses_empty_error_message() {
            let resp: Response = "ERR".parse().unwrap();
            assert_eq!(resp, Response::Error("".to_string()));
        }

        #[test]
        fn parses_error_with_trailing_space() {
            let resp: Response = "ERR ".parse().unwrap();
            assert_eq!(resp, Response::Error("".to_string()));
        }
    }
}
