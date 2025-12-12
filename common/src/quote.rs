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
    /// Creates a new stock quote with the current timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The ticker is empty
    /// - System time is before UNIX epoch
    /// - Timestamp overflows u64
    pub fn new(
        ticker: impl Into<String>,
        price: Decimal,
        volume: u32,
    ) -> Result<Self> {
        let ticker = ticker.into();

        if ticker.is_empty() {
            return Err(anyhow!("Ticker cannot be empty"));
        }

        let timestamp = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("System time before UNIX epoch")?
                .as_millis(),
        )?;

        Ok(Self {
            ticker,
            price,
            volume,
            timestamp,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

impl fmt::Display for StockQuote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match serde_json::to_string(self) {
            Ok(json) => write!(f, "{json}"),
            Err(_) => write!(f, "StockQuote{{ {} }}", self.ticker),
        }
    }
}

impl FromStr for StockQuote {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("Invalid JSON quote")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::any;
    use proptest::strategy::Strategy;
    use proptest::{prop_assert, prop_assert_eq, proptest};

    fn valid_ticker() -> impl Strategy<Value = String> {
        "[A-Z]{1,10}"
    }

    fn valid_price() -> impl Strategy<Value = Decimal> {
        (1i64..1_000_000i64, 0u32..4u32)
            .prop_map(|(mantissa, scale)| Decimal::new(mantissa, scale))
    }

    fn valid_quote() -> impl Strategy<Value = StockQuote> {
        (valid_ticker(), valid_price(), any::<u32>(), any::<u64>()).prop_map(
            |(ticker, price, volume, timestamp)| StockQuote {
                ticker,
                price,
                volume,
                timestamp,
            },
        )
    }

    proptest! {
        #[test]
        fn json_roundtrip(quote in valid_quote()) {
            let json = quote.to_string();
            let parsed: StockQuote = json.parse().unwrap();
            prop_assert_eq!(quote, parsed);
        }

        #[test]
        fn to_bytes_roundtrip(quote in valid_quote()) {
            let bytes = quote.to_bytes();
            let parsed: StockQuote = serde_json::from_slice(&bytes).unwrap();
            prop_assert_eq!(quote, parsed);
        }

        #[test]
        fn rejects_empty_ticker(
            price in valid_price(),
            volume in any::<u32>(),
        ) {
            prop_assert!(StockQuote::new("", price, volume).is_err());
        }
    }

    #[test]
    fn parses_json() {
        let json = r#"{"ticker":"AAPL","price":"150.50","volume":1000,"timestamp":1234567890}"#;
        let quote: StockQuote = json.parse().unwrap();
        assert_eq!(quote.ticker, "AAPL");
        assert_eq!(quote.volume, 1000);
    }

    #[test]
    fn display_is_json() {
        let quote = StockQuote {
            ticker: "TSLA".to_string(),
            price: Decimal::new(42050, 2),
            volume: 500,
            timestamp: 1_700_000_000,
        };

        let display = quote.to_string();
        assert!(display.starts_with('{'));
        assert!(display.contains("\"ticker\":\"TSLA\""));
    }

    #[test]
    fn rejects_invalid_json() {
        assert!("not json".parse::<StockQuote>().is_err());
        assert!("{}".parse::<StockQuote>().is_err());
    }
}
