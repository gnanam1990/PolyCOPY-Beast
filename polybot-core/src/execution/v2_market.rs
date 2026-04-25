use polybot_common::errors::PolybotError;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum V2FeeShape {
    Curve {
        rate: f64,
        exponent: f64,
        taker_only: bool,
    },
    BaseFees {
        maker_base_fee_bps: f64,
        taker_base_fee_bps: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct V2MarketInfoSnapshot {
    pub condition_id: String,
    pub token_count: usize,
    pub minimum_order_size: f64,
    pub minimum_tick_size: f64,
    pub fee_shape: V2FeeShape,
}

fn number_at(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_f64))
}

fn parse_fee_shape(value: &Value) -> Result<V2FeeShape, PolybotError> {
    if let Some(fd) = value.get("fd") {
        let rate = number_at(fd, &["r"]).ok_or_else(|| {
            PolybotError::Execution("V2 market info fd.r fee rate missing".to_string())
        })?;
        let exponent = number_at(fd, &["e"]).ok_or_else(|| {
            PolybotError::Execution("V2 market info fd.e fee exponent missing".to_string())
        })?;
        let taker_only = fd.get("to").and_then(Value::as_bool).ok_or_else(|| {
            PolybotError::Execution("V2 market info fd.to taker-only flag missing".to_string())
        })?;
        return Ok(V2FeeShape::Curve {
            rate,
            exponent,
            taker_only,
        });
    }

    let maker_base_fee_bps = number_at(value, &["mbf", "maker_base_fee"]).ok_or_else(|| {
        PolybotError::Execution(
            "V2 market info missing maker fee fields: expected fd or maker_base_fee/mbf"
                .to_string(),
        )
    })?;
    let taker_base_fee_bps = number_at(value, &["tbf", "taker_base_fee"]).ok_or_else(|| {
        PolybotError::Execution(
            "V2 market info missing taker fee fields: expected fd or taker_base_fee/tbf"
                .to_string(),
        )
    })?;
    Ok(V2FeeShape::BaseFees {
        maker_base_fee_bps,
        taker_base_fee_bps,
    })
}

pub fn verify_v2_market_info_shape(value: &Value) -> Result<V2MarketInfoSnapshot, PolybotError> {
    let condition_id = value
        .get("condition_id")
        .or_else(|| value.get("conditionID"))
        .and_then(Value::as_str)
        .ok_or_else(|| PolybotError::Execution("V2 market info condition_id missing".to_string()))?
        .to_string();
    let token_count = value
        .get("tokens")
        .or_else(|| value.get("t"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| PolybotError::Execution("V2 market info tokens missing".to_string()))?;
    if token_count == 0 {
        return Err(PolybotError::Execution(
            "V2 market info tokens array is empty".to_string(),
        ));
    }

    let minimum_order_size = number_at(value, &["mos", "minimum_order_size"]).ok_or_else(|| {
        PolybotError::Execution("V2 market info minimum order size missing".to_string())
    })?;
    let minimum_tick_size = number_at(value, &["mts", "minimum_tick_size"]).ok_or_else(|| {
        PolybotError::Execution("V2 market info minimum tick size missing".to_string())
    })?;
    let fee_shape = parse_fee_shape(value)?;

    Ok(V2MarketInfoSnapshot {
        condition_id,
        token_count,
        minimum_order_size,
        minimum_tick_size,
        fee_shape,
    })
}

pub async fn fetch_and_verify_v2_market_info(
    endpoint: &str,
    condition_id: &str,
) -> Result<V2MarketInfoSnapshot, PolybotError> {
    let endpoint = endpoint.trim_end_matches('/');
    let url = format!("{}/markets/{}", endpoint, condition_id);
    let value = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| PolybotError::Execution(format!("Failed to create V2 verifier: {}", e)))?
        .get(&url)
        .send()
        .await
        .map_err(|e| PolybotError::Execution(format!("V2 market info fetch failed: {}", e)))?
        .error_for_status()
        .map_err(|e| PolybotError::Execution(format!("V2 market info fetch failed: {}", e)))?
        .json::<Value>()
        .await
        .map_err(|e| PolybotError::Execution(format!("V2 market info parse failed: {}", e)))?;

    verify_v2_market_info_shape(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_current_public_market_shape() {
        let value = serde_json::json!({
            "condition_id": "0xabc",
            "tokens": [
                {"token_id": "1", "outcome": "Yes"},
                {"token_id": "2", "outcome": "No"}
            ],
            "minimum_order_size": 5,
            "minimum_tick_size": 0.01,
            "maker_base_fee": 0,
            "taker_base_fee": 25
        });

        let verified = verify_v2_market_info_shape(&value).unwrap();

        assert_eq!(verified.condition_id, "0xabc");
        assert_eq!(verified.token_count, 2);
        assert_eq!(
            verified.fee_shape,
            V2FeeShape::BaseFees {
                maker_base_fee_bps: 0.0,
                taker_base_fee_bps: 25.0,
            }
        );
    }

    #[test]
    fn verifies_clob_market_info_fee_curve_shape() {
        let value = serde_json::json!({
            "conditionID": "0xabc",
            "t": [
                {"t": "1", "o": "Yes"},
                {"t": "2", "o": "No"}
            ],
            "mos": 5,
            "mts": 0.01,
            "fd": {"r": 0.02, "e": 2, "to": true}
        });

        let verified = verify_v2_market_info_shape(&value).unwrap();

        assert_eq!(verified.token_count, 2);
        assert_eq!(
            verified.fee_shape,
            V2FeeShape::Curve {
                rate: 0.02,
                exponent: 2.0,
                taker_only: true,
            }
        );
    }

    #[test]
    fn rejects_missing_fee_shape() {
        let value = serde_json::json!({
            "condition_id": "0xabc",
            "tokens": [{"token_id": "1", "outcome": "Yes"}],
            "minimum_order_size": 5,
            "minimum_tick_size": 0.01
        });

        assert!(verify_v2_market_info_shape(&value).is_err());
    }
}
