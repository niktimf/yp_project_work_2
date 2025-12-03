use anyhow::{anyhow, Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockQuote {
    pub ticker: String,
    pub price: Decimal,
    pub volume: u32,
    pub timestamp: u64,
}

impl StockQuote {
    const FIELD_SEPARATOR: char = '|';
    pub fn new(
        ticker: impl Into<String>,
        price: Decimal,
        volume: u32,
    ) -> Result<Self> {
        let ticker = ticker.into();

        if ticker.is_empty() {
            return Err(anyhow!("Ticker cannot be empty"));
        }
        if ticker.contains(Self::FIELD_SEPARATOR) {
            return Err(anyhow!(
                "Ticker cannot contain '{sep}'",
                sep = Self::FIELD_SEPARATOR
            ));
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time before UNIX epoch")?
            .as_millis() as u64;

        Ok(Self {
            ticker,
            price,
            volume,
            timestamp,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl fmt::Display for StockQuote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{1}{0}{2}{0}{3}{0}{4}",
            Self::FIELD_SEPARATOR,
            self.ticker,
            self.price,
            self.volume,
            self.timestamp
        )
    }
}

impl FromStr for StockQuote {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut parts = s.split(Self::FIELD_SEPARATOR);

        let ticker = parts.next().ok_or(anyhow!("Missing ticker"))?.to_string();

        let price = parts
            .next()
            .ok_or(anyhow!("Missing price"))?
            .parse()
            .context("Invalid price")?;

        let volume = parts
            .next()
            .ok_or(anyhow!("Missing volume"))?
            .parse()
            .context("Invalid volume")?;

        let timestamp = parts
            .next()
            .ok_or(anyhow!("Missing timestamp"))?
            .parse()
            .context("Invalid timestamp")?;

        if parts.next().is_some() {
            return Err(anyhow!("Too many fields in quote"));
        }

        Ok(Self {
            ticker,
            price,
            volume,
            timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use super::*;

    #[test]
    fn roundtrip_serialization() {
        let quote = StockQuote {
            ticker: "AAPL".to_string(),
            price: Decimal::new(15050, 2), // 150.50
            volume: 1000,
            timestamp: 1234567890,
        };

        let serialized = quote.to_string();
        let parsed: StockQuote = serialized.parse().unwrap();

        assert_eq!(quote, parsed);
    }

    #[test]
    fn rejects_empty_ticker() {
        let result = StockQuote::new("", Decimal::ONE, 100);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_ticker_with_separator() {
        let result = StockQuote::new("AA|PL", Decimal::ONE, 100);
        assert!(result.is_err());
    }

    #[rstest]
    #[case("AAPL|150")]
    #[case("AAPL|100|50|123|extra")]
    fn rejects_malformed_input(#[case] input: &str) {
        assert!(input.parse::<StockQuote>().is_err());
    }
}
