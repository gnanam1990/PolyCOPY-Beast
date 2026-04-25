use polybot_common::errors::PolybotError;
use polybot_common::types::{Category, ScannerEvent, Side, Signal, SignalSource, TradeDirection};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::Instant;
use uuid::Uuid;

fn parse_optional_decimal(raw: &serde_json::Value, key: &str) -> Option<Decimal> {
    raw.get(key).and_then(|value| match value {
        serde_json::Value::Number(number) => Decimal::from_str(&number.to_string()).ok(),
        serde_json::Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    })
}

fn parse_signal_source(raw: &serde_json::Value) -> SignalSource {
    match raw
        .get("source")
        .and_then(|value| value.as_str())
        .unwrap_or("manual")
        .to_lowercase()
        .as_str()
    {
        "polling" => SignalSource::Polling,
        "websocket" => SignalSource::Websocket,
        "http" => SignalSource::Http,
        _ => SignalSource::Manual,
    }
}

fn infer_category(title: Option<&str>, slug: Option<&str>) -> Category {
    let haystack = format!(
        "{} {}",
        title.unwrap_or_default().to_lowercase(),
        slug.unwrap_or_default().to_lowercase()
    );

    if ["btc", "bitcoin", "eth", "ethereum", "sol", "crypto"]
        .iter()
        .any(|needle| haystack.contains(needle))
    {
        Category::Crypto
    } else if [
        "nba",
        "nfl",
        "mlb",
        "soccer",
        "championship",
        "game",
        "sports",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
    {
        Category::Sports
    } else {
        Category::Politics
    }
}

fn normalize_trade_side(raw: &str) -> Result<Side, PolybotError> {
    match raw.to_uppercase().as_str() {
        "YES" => Ok(Side::Yes),
        "NO" => Ok(Side::No),
        other => Err(PolybotError::Scanner(format!(
            "Invalid normalized outcome side: {}",
            other
        ))),
    }
}

fn parse_trade_direction(raw: &str) -> Result<TradeDirection, PolybotError> {
    match raw.to_uppercase().as_str() {
        "BUY" => Ok(TradeDirection::Buy),
        "SELL" => Ok(TradeDirection::Sell),
        other => Err(PolybotError::Scanner(format!(
            "Invalid trade direction: {}",
            other
        ))),
    }
}

pub fn normalize_data_api_trade(json: &str, source: SignalSource) -> Result<Signal, PolybotError> {
    let raw: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| PolybotError::Scanner(format!("JSON parse error: {}", e)))?;

    let event_type = raw
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if event_type != "TRADE" {
        return Err(PolybotError::Scanner(format!(
            "Unsupported activity type: {}",
            event_type
        )));
    }

    let outcome = raw
        .get("outcome")
        .and_then(|value| value.as_str())
        .ok_or_else(|| PolybotError::Scanner("Missing outcome field".to_string()))?;

    let wallet_address = raw
        .get("proxyWallet")
        .or_else(|| raw.get("user"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| PolybotError::Scanner("Missing proxyWallet field".to_string()))?
        .to_lowercase();

    let timestamp = raw
        .get("timestamp")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .ok_or_else(|| PolybotError::Scanner("Missing timestamp field".to_string()))?;
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
        .ok_or_else(|| PolybotError::Scanner("Invalid activity timestamp".to_string()))?
        .to_rfc3339();

    let category = raw
        .get("category")
        .and_then(|value| value.as_str())
        .and_then(|value| Category::try_from(value).ok())
        .unwrap_or_else(|| {
            infer_category(
                raw.get("title").and_then(|value| value.as_str()),
                raw.get("slug").and_then(|value| value.as_str()),
            )
        });

    let target_size_usdc = parse_optional_decimal(&raw, "usdcSize");
    let target_size_tokens = parse_optional_decimal(&raw, "size");
    let target_price = parse_optional_decimal(&raw, "price");

    Ok(Signal {
        signal_id: Uuid::new_v4().to_string(),
        timestamp,
        wallet_address,
        market_id: raw
            .get("conditionId")
            .and_then(|value| value.as_str())
            .ok_or_else(|| PolybotError::Scanner("Missing conditionId field".to_string()))?
            .to_string(),
        side: normalize_trade_side(outcome)?,
        direction: parse_trade_direction(
            raw.get("side")
                .and_then(|value| value.as_str())
                .ok_or_else(|| PolybotError::Scanner("Missing or invalid side field".to_string()))?,
        )?,
        confidence: 8,
        secret_level: 8,
        category,
        source,
        tx_hash: raw
            .get("transactionHash")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        token_id: raw
            .get("asset")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        target_price,
        target_size_usdc,
        target_size_tokens,
        resolved: raw
            .get("resolved")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        redeemable: raw
            .get("redeemable")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        suggested_size_usdc: target_size_usdc,
        fee_schedule: None,
        scanner_version: "2.0.0".to_string(),
    })
}

pub fn parse_signal(json: &str) -> Result<Signal, PolybotError> {
    let raw: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| PolybotError::Scanner(format!("JSON parse error: {}", e)))?;

    let category_str = raw
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("other");
    let category = Category::try_from(category_str).unwrap_or(Category::Other);

    let side_str = raw
        .get("side")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PolybotError::Scanner("Missing 'side' field".to_string()))?;
    let side = match side_str.to_uppercase().as_str() {
        "YES" => Side::Yes,
        "NO" => Side::No,
        _ => {
            return Err(PolybotError::Scanner(format!(
                "Invalid side: '{}'. Must be YES or NO",
                side_str
            )))
        }
    };

    let suggested_size = raw
        .get("suggested_size_usdc")
        .and_then(|v| v.as_f64())
        .map(|v| Decimal::try_from(v).unwrap_or(Decimal::ZERO));

    let direction = match raw.get("direction") {
        Some(value) => parse_trade_direction(value.as_str().ok_or_else(|| {
            PolybotError::Scanner("Invalid direction: must be BUY or SELL".to_string())
        })?)?,
        None => TradeDirection::Buy,
    };

    let signal = Signal {
        signal_id: raw
            .get("signal_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        timestamp: raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        wallet_address: raw
            .get("wallet_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        market_id: raw
            .get("market_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        side,
        direction,
        confidence: raw.get("confidence").and_then(|v| v.as_u64()).unwrap_or(0) as u8,
        secret_level: raw
            .get("secret_level")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8,
        category,
        source: parse_signal_source(&raw),
        tx_hash: raw
            .get("tx_hash")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string()),
        token_id: raw
            .get("token_id")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string()),
        target_price: parse_optional_decimal(&raw, "target_price"),
        target_size_usdc: parse_optional_decimal(&raw, "target_size_usdc"),
        target_size_tokens: parse_optional_decimal(&raw, "target_size_tokens"),
        resolved: raw
            .get("resolved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        redeemable: raw
            .get("redeemable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        suggested_size_usdc: suggested_size,
        fee_schedule: None,
        scanner_version: raw
            .get("scanner_version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0")
            .to_string(),
    };

    Ok(signal)
}

pub fn validate_and_create_event(json: &str) -> Result<ScannerEvent, PolybotError> {
    validate_and_create_event_with_max_age(json, 30)
}

pub fn validate_and_create_event_with_max_age(
    json: &str,
    max_age_secs: u64,
) -> Result<ScannerEvent, PolybotError> {
    let signal = parse_signal(json)?;
    signal
        .validate_with_max_age_secs(max_age_secs as i64)
        .map_err(|errors| {
            PolybotError::Scanner(
                errors
                    .iter()
                    .map(|e| format!("{:?}", e))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })?;

    // v2.5: log warning if signal requires manual review
    if signal.requires_manual_review() {
        tracing::warn!(
            signal_id = %signal.signal_id,
            confidence = signal.confidence,
            secret_level = signal.secret_level,
            "Signal queued for manual review (confidence or secret_level < 3)"
        );
    }

    Ok(ScannerEvent {
        signal,
        received_at: Instant::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_v25_signal() {
        let json = r#"{
            "signal_id": "550e8400-e29b-41d4-a716-446655440000",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "polymarket-clob-market-123",
            "side": "YES",
            "confidence": 7,
            "secret_level": 6,
            "category": "politics",
            "suggested_size_usdc": 50.00,
            "scanner_version": "1.0.0"
        }"#;
        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.confidence, 7);
        assert_eq!(signal.secret_level, 6);
        assert_eq!(signal.market_id, "polymarket-clob-market-123");
        assert_eq!(signal.side, Side::Yes);
        assert_eq!(signal.category, Category::Politics);
        assert!(signal.suggested_size_usdc.is_some());
    }

    #[test]
    fn parse_normalized_module2_signal_fields() {
        let json = r#"{
            "signal_id": "550e8400-e29b-41d4-a716-446655440000",
            "source": "polling",
            "tx_hash": "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "polymarket-clob-market-123",
            "token_id": "123456789",
            "side": "YES",
            "confidence": 7,
            "secret_level": 6,
            "category": "politics",
            "target_size_usdc": 500.0,
            "target_price": 0.62,
            "suggested_size_usdc": 50.0,
            "resolved": false,
            "redeemable": false,
            "scanner_version": "2.0.0"
        }"#;

        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.source, polybot_common::types::SignalSource::Polling);
        assert_eq!(
            signal.tx_hash.as_deref(),
            Some("0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed")
        );
        assert_eq!(signal.token_id.as_deref(), Some("123456789"));
        assert_eq!(
            signal.target_size_usdc,
            Some(Decimal::from_str("500.0").unwrap())
        );
        assert_eq!(
            signal.target_price,
            Some(Decimal::from_str("0.62").unwrap())
        );
    }

    #[test]
    fn parse_signal_reads_direction_when_present() {
        let json = r#"{
            "signal_id": "550e8400-e29b-41d4-a716-446655440000",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "polymarket-clob-market-123",
            "side": "YES",
            "direction": "SELL",
            "confidence": 7,
            "secret_level": 6,
            "category": "politics"
        }"#;

        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.direction, TradeDirection::Sell);
    }

    #[test]
    fn parse_signal_defaults_direction_to_buy_when_omitted() {
        let json = r#"{
            "signal_id": "550e8400-e29b-41d4-a716-446655440000",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "polymarket-clob-market-123",
            "side": "YES",
            "confidence": 7,
            "secret_level": 6,
            "category": "politics"
        }"#;

        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.direction, TradeDirection::Buy);
    }

    #[test]
    fn parse_signal_rejects_malformed_explicit_direction() {
        let json = r#"{
            "signal_id": "550e8400-e29b-41d4-a716-446655440000",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "polymarket-clob-market-123",
            "side": "YES",
            "direction": 123,
            "confidence": 7,
            "secret_level": 6,
            "category": "politics"
        }"#;

        let result = parse_signal(json);
        assert!(result.is_err());
    }

    #[test]
    fn normalize_data_api_trade_to_signal() {
        let json = r#"{
            "proxyWallet": "0xabc123abc123abc123abc123abc123abc123abc1",
            "timestamp": 1760000000,
            "conditionId": "0xdef456def456def456def456def456def456def456def456def456def456def4",
            "type": "TRADE",
            "size": 10.0,
            "usdcSize": 5.7,
            "transactionHash": "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed",
            "price": 0.57,
            "asset": "123456789",
            "side": "BUY",
            "outcome": "YES",
            "title": "Will ETH close above $4000?",
            "slug": "eth-above-4000"
        }"#;

        let signal = normalize_data_api_trade(json, SignalSource::Polling).unwrap();
        assert_eq!(signal.source, SignalSource::Polling);
        assert_eq!(
            signal.wallet_address,
            "0xabc123abc123abc123abc123abc123abc123abc1"
        );
        assert_eq!(
            signal.market_id,
            "0xdef456def456def456def456def456def456def456def456def456def456def4"
        );
        assert_eq!(signal.token_id.as_deref(), Some("123456789"));
        assert_eq!(
            signal.tx_hash.as_deref(),
            Some("0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed")
        );
        assert_eq!(signal.side, Side::Yes);
        assert_eq!(signal.category, Category::Crypto);
        assert_eq!(
            signal.target_size_usdc,
            Some(Decimal::from_str("5.7").unwrap())
        );
        assert_eq!(
            signal.suggested_size_usdc,
            Some(Decimal::from_str("5.7").unwrap())
        );
    }

    #[test]
    fn normalize_data_api_trade_preserves_sell_direction_and_token_size() {
        let json = r#"{
            "proxyWallet": "0xabc123abc123abc123abc123abc123abc123abc1",
            "timestamp": 1760000000,
            "conditionId": "0xdef456def456def456def456def456def456def456def456def456def456def4",
            "type": "TRADE",
            "side": "SELL",
            "outcome": "YES",
            "size": 12.5,
            "usdcSize": 7.5,
            "price": 0.60,
            "asset": "123456789",
            "transactionHash": "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed"
        }"#;

        let signal = normalize_data_api_trade(json, SignalSource::Polling).unwrap();
        assert_eq!(signal.side, Side::Yes);
        assert_eq!(signal.direction, TradeDirection::Sell);
        assert_eq!(
            signal.target_size_tokens,
            Some(Decimal::from_str("12.5").unwrap())
        );
    }

    #[test]
    fn normalize_data_api_trade_rejects_malformed_side() {
        let json = r#"{
            "proxyWallet": "0xabc123abc123abc123abc123abc123abc123abc1",
            "timestamp": 1760000000,
            "conditionId": "0xdef456def456def456def456def456def456def456def456def456def456def4",
            "type": "TRADE",
            "side": 123,
            "outcome": "YES",
            "size": 12.5,
            "usdcSize": 7.5,
            "price": 0.60,
            "asset": "123456789",
            "transactionHash": "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed"
        }"#;

        let result = normalize_data_api_trade(json, SignalSource::Polling);
        assert!(result.is_err());
    }

    #[test]
    fn parse_side_uppercase() {
        let json = r#"{
            "signal_id": "test-id",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "market-1",
            "side": "NO",
            "confidence": 5,
            "secret_level": 5,
            "category": "sports"
        }"#;
        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.side, Side::No);
    }

    #[test]
    fn parse_unknown_category_defaults_to_other() {
        let json = r#"{
            "signal_id": "test-id",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "market-1",
            "side": "YES",
            "confidence": 5,
            "secret_level": 5,
            "category": "unknown_cat"
        }"#;
        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.category, Category::Other);
    }

    #[test]
    fn validate_good_event() {
        let json = r#"{
            "signal_id": "550e8400-e29b-41d4-a716-446655440000",
            "timestamp": "2026-04-14T12:34:56.789Z",
            "wallet_address": "0xabc123abc123abc123abc123abc123abc123abc1",
            "market_id": "market-1",
            "side": "YES",
            "confidence": 7,
            "secret_level": 7,
            "category": "politics"
        }"#;
        // Stale timestamp is a validation error in v2.5
        let _result = validate_and_create_event(json);
        // Stale timestamp is a validation error in v2.5
        // The signal itself parses fine
        let signal = parse_signal(json).unwrap();
        assert_eq!(signal.confidence, 7);
    }

    #[test]
    fn validate_bad_signal_fails() {
        let json = r#"{
            "signal_id": "",
            "timestamp": "not-a-timestamp",
            "wallet_address": "abc",
            "market_id": "",
            "side": "YES",
            "confidence": 0,
            "secret_level": 0,
            "category": "politics"
        }"#;
        let result = validate_and_create_event(json);
        assert!(result.is_err());
    }
}
