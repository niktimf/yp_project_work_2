use anyhow::Result;
use rand::rngs::ThreadRng;
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use common::{StockQuote, Tickers};

pub struct QuoteGenerator {
    prices: HashMap<String, Decimal>,
    rng: ThreadRng,
}

impl QuoteGenerator {
    const MAX_PRICE_CHANGE_PERCENT: Decimal = dec!(0.01);

    const TICKER_MIN_PRICE: Decimal = dec!(1.00);

    const UNKNOWN_TICKER_DEFAULT_PRICE: Decimal = dec!(100.00);

    const RANDOM_PRICE_MIN: Decimal = dec!(50.00);
    const RANDOM_PRICE_MAX: Decimal = dec!(500.00);

    const HIGH_VOLUME_TICKERS: &[&str] =
        &["AAPL", "MSFT", "TSLA", "NVDA", "GOOGL", "AMZN", "META"];

    const HIGH_VOLUME_BASE: u32 = 1000;
    const HIGH_VOLUME_RANGE: u32 = 5000;
    const NORMAL_VOLUME_BASE: u32 = 100;
    const NORMAL_VOLUME_RANGE: u32 = 1000;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prices(prices: HashMap<String, Decimal>) -> Self {
        Self {
            prices,
            rng: rand::thread_rng(),
        }
    }

    pub fn with_tickers(tickers: &Tickers) -> Self {
        let mut rng = rand::thread_rng();
        let prices = tickers
            .iter()
            .map(|ticker| {
                let price = random_price(&mut rng);
                (ticker.to_string(), price)
            })
            .collect();

        Self { prices, rng }
    }

    pub fn generate(&mut self, ticker: &str) -> Result<StockQuote> {
        let price = self.next_price(ticker);
        let volume = self.random_volume(ticker);

        StockQuote::new(ticker, price, volume)
    }

    pub fn generate_batch(
        &mut self,
        tickers: &Tickers,
    ) -> Result<Vec<StockQuote>> {
        tickers.iter().map(|ticker| self.generate(ticker)).collect()
    }

    #[must_use]
    pub fn current_price(&self, ticker: &str) -> Option<Decimal> {
        self.prices.get(ticker).copied()
    }

    #[must_use]
    pub fn known_tickers(&self) -> Vec<&str> {
        self.prices.keys().map(String::as_str).collect()
    }

    fn next_price(&mut self, ticker: &str) -> Decimal {
        let price = self
            .prices
            .entry(ticker.to_string())
            .or_insert(Self::UNKNOWN_TICKER_DEFAULT_PRICE);

        let change_factor = random_change_factor(&mut self.rng);
        *price = (*price * change_factor).max(Self::TICKER_MIN_PRICE);
        *price
    }

    fn random_volume(&mut self, ticker: &str) -> u32 {
        let (base, range) = if is_high_volume_ticker(ticker) {
            (Self::HIGH_VOLUME_BASE, Self::HIGH_VOLUME_RANGE)
        } else {
            (Self::NORMAL_VOLUME_BASE, Self::NORMAL_VOLUME_RANGE)
        };

        base + self.rng.gen_range(0..range)
    }
}

impl Default for QuoteGenerator {
    fn default() -> Self {
        let prices = HashMap::from([
            ("AAPL".to_string(), dec!(285.00)),
            ("GOOGL".to_string(), dec!(315.00)),
            ("TSLA".to_string(), dec!(425.00)),
            ("MSFT".to_string(), dec!(490.00)),
            ("AMZN".to_string(), dec!(235.00)),
            ("NVDA".to_string(), dec!(180.00)),
            ("META".to_string(), dec!(640.00)),
        ]);

        Self::with_prices(prices)
    }
}

fn is_high_volume_ticker(ticker: &str) -> bool {
    QuoteGenerator::HIGH_VOLUME_TICKERS.contains(&ticker)
}

fn random_change_factor(rng: &mut ThreadRng) -> Decimal {
    // Change factor from (1 - MAX) to (1 + MAX)
    let change = random_decimal(
        rng,
        -QuoteGenerator::MAX_PRICE_CHANGE_PERCENT,
        QuoteGenerator::MAX_PRICE_CHANGE_PERCENT,
    );
    Decimal::ONE + change
}

fn random_decimal(rng: &mut ThreadRng, min: Decimal, max: Decimal) -> Decimal {
    let range = max - min;
    let random_factor =
        Decimal::from(rng.gen_range(0_u32..10000)) / dec!(10000);
    min + range * random_factor
}

fn random_price(rng: &mut ThreadRng) -> Decimal {
    random_decimal(
        rng,
        QuoteGenerator::RANDOM_PRICE_MIN,
        QuoteGenerator::RANDOM_PRICE_MAX,
    )
    .round_dp(2)
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    fn valid_ticker() -> impl Strategy<Value = String> {
        "[A-Z]{1,5}"
    }

    proptest! {
        #[test]
        fn price_never_below_minimum(ticker in valid_ticker()) {
            let mut gen = QuoteGenerator::default();

            for _ in 0..1000 {
                let quote = gen.generate(&ticker).unwrap();
                prop_assert!(quote.price >= QuoteGenerator::TICKER_MIN_PRICE);
            }
        }

        #[test]
        fn price_changes_bounded(ticker in valid_ticker()) {
            let mut gen = QuoteGenerator::default();
            let first = gen.generate(&ticker).unwrap();
            let second = gen.generate(&ticker).unwrap();
            let max_change = first.price * QuoteGenerator::MAX_PRICE_CHANGE_PERCENT;
            let actual_change = (second.price - first.price).abs();
            prop_assert!(actual_change <= max_change + dec!(0.01));
        }

        #[test]
        fn generates_valid_quotes(ticker in valid_ticker()) {
            let mut gen = QuoteGenerator::default();
            let quote = gen.generate(&ticker).unwrap();

            prop_assert_eq!(quote.ticker, ticker);
            prop_assert!(quote.volume > 0);
        }

        #[test]
        fn remembers_price(ticker in valid_ticker()) {
            let mut gen = QuoteGenerator::default();

            let _ = gen.generate(&ticker).unwrap();
            let stored = gen.current_price(&ticker);

            prop_assert!(stored.is_some());
        }
    }

    #[rstest]
    #[case("AAPL", true)]
    #[case("TSLA", true)]
    #[case("UNKNOWN", false)]
    fn default_has_standard_tickers(#[case] ticker: &str, #[case] exists: bool) {
        let gen = QuoteGenerator::default();
        assert_eq!(gen.current_price(ticker).is_some(), exists);
    }

    #[rstest]
    #[case("AAA")]
    #[case("BBB")]
    #[case("CCC")]
    fn with_tickers_initializes_all(#[case] ticker: &str) {
        let tickers: Tickers = "AAA,BBB,CCC".parse().unwrap();
        let gen = QuoteGenerator::with_tickers(&tickers);
        assert!(gen.current_price(ticker).is_some());
    }

    #[test]
    fn high_volume_tickers_have_more_volume() {
        let mut gen = QuoteGenerator::default();
        let mut high_volumes = Vec::new();
        let mut normal_volumes = Vec::new();

        for _ in 0..100 {
            high_volumes.push(gen.generate("AAPL").unwrap().volume);
            normal_volumes.push(gen.generate("UNKNOWN").unwrap().volume);
        }

        let high_avg: u32 = high_volumes.iter().sum::<u32>() / 100;
        let normal_avg: u32 = normal_volumes.iter().sum::<u32>() / 100;

        assert!(high_avg > normal_avg);
    }

    #[rstest]
    #[case("AAPL,TSLA,META", 3, &["AAPL", "TSLA", "META"])]
    #[case("GOOGL,AMZN", 2, &["GOOGL", "AMZN"])]
    fn generate_batch_returns_all(
        #[case] input: &str,
        #[case] expected_len: usize,
        #[case] expected_tickers: &[&str],
    ) {
        let mut gen = QuoteGenerator::default();
        let tickers: Tickers = input.parse().unwrap();

        let quotes = gen.generate_batch(&tickers).unwrap();

        assert_eq!(quotes.len(), expected_len);
        for ticker in expected_tickers {
            assert!(quotes.iter().any(|q| q.ticker == *ticker));
        }
    }

    #[rstest]
    #[case(dec!(10), dec!(20))]
    #[case(dec!(0), dec!(100))]
    #[case(dec!(-50), dec!(50))]
    fn random_decimal_in_range(#[case] min: Decimal, #[case] max: Decimal) {
        let mut rng = rand::thread_rng();

        for _ in 0..1000 {
            let val = random_decimal(&mut rng, min, max);
            assert!(val >= min);
            assert!(val < max);
        }
    }
}
