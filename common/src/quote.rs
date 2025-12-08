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

    pub fn new(ticker: impl Into<String>, price: Decimal, volume: u32) -> Result<Self> {
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
    use super::*;
    use proptest::prelude::{any, prop};
    use proptest::strategy::Strategy;
    use proptest::{prop_assert, prop_assert_eq, proptest};
    use rstest::rstest;

    #[test]
    fn roundtrip_serialization() {
        let quote = StockQuote {
            ticker: "AAPL".to_string(),
            price: Decimal::ONE_HUNDRED,
            volume: 1000,
            timestamp: 1234567890,
        };

        let serialized = quote.to_string();
        let parsed: StockQuote = serialized.parse().unwrap();

        assert_eq!(quote, parsed);
    }

    #[rstest]
    #[case("AAPL|150")]
    #[case("AAPL|100|50|123|extra")]
    fn rejects_malformed_input(#[case] input: &str) {
        assert!(input.parse::<StockQuote>().is_err());
    }

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
        fn roundtrip(quote in valid_quote()) {
            let serialized = quote.to_string();
            let parsed: StockQuote = serialized.parse().unwrap();
            prop_assert_eq!(quote, parsed);
        }

        #[test]
        fn display_has_four_fields(quote in valid_quote()) {
            let serialized = quote.to_string();
            let parts: Vec<_> = serialized.split(StockQuote::FIELD_SEPARATOR).collect();
            prop_assert_eq!(parts.len(), 4);
        }

        #[test]
        fn to_bytes_matches_display(quote in valid_quote()) {
            prop_assert_eq!(quote.to_bytes(), quote.to_string().into_bytes());
        }

        #[test]
        fn rejects_empty_ticker(
            price in valid_price(),
            volume in any::<u32>(),
        ) {
            prop_assert!(StockQuote::new("", price, volume).is_err());
        }

        #[test]
        fn rejects_ticker_with_separator(
            before in "[A-Z]{0,5}",
            after in "[A-Z]{0,5}",
            price in valid_price(),
            volume in any::<u32>()
        ) {
            let separator = StockQuote::FIELD_SEPARATOR;
            let bad_ticker = format!("{before}{separator}{after}");
            prop_assert!(StockQuote::new(bad_ticker, price, volume).is_err());
        }

        #[test]
        fn rejects_wrong_field_count(
            parts in prop::collection::vec("[^|]+", 1..10_usize)
        ) {
            let input = parts.join(&StockQuote::FIELD_SEPARATOR.to_string());
            if parts.len() != 4 {
                prop_assert!(input.parse::<StockQuote>().is_err());
            }
        }
    }

    #[test]
    fn parses_extreme_values() {
        let quote = StockQuote {
            ticker: "X".to_string(),
            price: Decimal::MAX,
            volume: u32::MAX,
            timestamp: u64::MAX,
        };

        let parsed: StockQuote = quote.to_string().parse().unwrap();
        assert_eq!(quote, parsed);
    }

    #[test]
    fn parses_zero_values() {
        let quote = StockQuote {
            ticker: "A".to_string(),
            price: Decimal::ZERO,
            volume: 0,
            timestamp: 0,
        };

        let parsed: StockQuote = quote.to_string().parse().unwrap();
        assert_eq!(quote, parsed);
    }
}
