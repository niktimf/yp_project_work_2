use anyhow::Result;
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::str::FromStr;

use crate::quote::StockQuote;

pub struct QuoteGenerator {
    prices: HashMap<String, Decimal>,
}

impl QuoteGenerator {
    pub fn new() -> Self {
        let mut prices = HashMap::new();
        prices.insert("AAPL".to_string(), dec!(285.00));
        prices.insert("GOOGL".to_string(), dec!(315.00));
        prices.insert("TSLA".to_string(), dec!(425.00));
        prices.insert("MSFT".to_string(), dec!(490.00));
        prices.insert("AMZN".to_string(), dec!(235.00));
        prices.insert("NVDA".to_string(), dec!(180.00));
        prices.insert("META".to_string(), dec!(640.00));

        QuoteGenerator { prices }
    }

    pub fn from_tickers(tickers: Vec<String>) -> Self {
        let mut prices = HashMap::new();
        let mut rng = rand::thread_rng();

        for ticker in tickers {
            let base_price = Decimal::from_str(&format!(
                "{:.2}",
                rng.gen_range(50.0..500.0)
            ))
            .unwrap_or(dec!(100.00));
            prices.insert(ticker, base_price);
        }

        QuoteGenerator { prices }
    }

    pub fn generate_quote(&mut self, ticker: &str) -> Result<StockQuote> {
        let mut rng = rand::thread_rng();

        let last_price = self
            .prices
            .entry(ticker.to_string())
            .or_insert(dec!(100.00));

        // Random walk: price changes by up to 1% in either direction
        let change_percent = rng.gen_range(-0.01..0.01);
        let change =
            *last_price * Decimal::from_str(&format!("{:.6}", change_percent))?;
        *last_price += change;

        // Ensure price doesn't go below 1.00
        if *last_price < dec!(1.00) {
            *last_price = dec!(1.00);
        }

        let volume = match ticker {
            "AAPL" | "MSFT" | "TSLA" | "NVDA" => 1000 + rng.gen_range(0..5000),
            _ => 100 + rng.gen_range(0..1000),
        };

        StockQuote::new(ticker.to_string(), *last_price, volume)
    }

    pub fn generate_quotes(
        &mut self,
        tickers: &[String],
    ) -> Vec<Result<StockQuote>> {
        tickers
            .iter()
            .map(|ticker| self.generate_quote(ticker))
            .collect()
    }
}
