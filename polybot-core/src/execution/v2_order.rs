use polybot_common::errors::PolybotError;
use polybot_common::types::{FeeSchedule, OrderType, TradeDirection};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuilderCode(pub [u8; 32]);

impl BuilderCode {
    pub fn as_hex(self) -> String {
        let mut out = String::with_capacity(66);
        out.push_str("0x");
        for byte in self.0 {
            out.push_str(&format!("{:02x}", byte));
        }
        out
    }
}

impl Serialize for BuilderCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_hex())
    }
}

impl<'de> Deserialize<'de> for BuilderCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_builder_code(&raw).map_err(serde::de::Error::custom)
    }
}

pub fn parse_builder_code(raw: &str) -> Result<BuilderCode, PolybotError> {
    let value = raw.trim();
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(PolybotError::Config(
            "BUILDER_CODE must be 0x-prefixed bytes32 hex".to_string(),
        ));
    };
    if !hex.is_ascii() {
        return Err(PolybotError::Config(
            "BUILDER_CODE contains non-ASCII hex".to_string(),
        ));
    }
    if hex.len() != 64 {
        return Err(PolybotError::Config(format!(
            "BUILDER_CODE must be 32 bytes, got {} hex chars",
            hex.len()
        )));
    }

    let mut out = [0u8; 32];
    for idx in 0..32 {
        let start = idx * 2;
        out[idx] = decode_hex_byte(&hex[start..start + 2])?;
    }
    Ok(BuilderCode(out))
}

fn decode_hex_byte(raw: &str) -> Result<u8, PolybotError> {
    u8::from_str_radix(raw, 16)
        .map_err(|_| PolybotError::Config(format!("BUILDER_CODE contains non-hex byte '{}'", raw)))
}

pub fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn clob_side(direction: TradeDirection) -> u8 {
    match direction {
        TradeDirection::Buy => 0,
        TradeDirection::Sell => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct V2OrderPayload {
    pub salt: u64,
    pub maker: String,
    pub signer: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub side: u8,
    pub signature_type: u8,
    pub timestamp_ms: u64,
    pub metadata: BuilderCode,
    pub builder: BuilderCode,
    pub order_type: OrderType,
    pub fee_schedule: Option<FeeSchedule>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_code_accepts_bytes32_hex() {
        let raw = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
        let parsed = parse_builder_code(raw).unwrap();
        assert_eq!(parsed.as_hex(), raw);
    }

    #[test]
    fn builder_code_serializes_as_hex_string() {
        let raw = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
        let parsed = parse_builder_code(raw).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::json!(raw)
        );
    }

    #[test]
    fn builder_code_deserializes_from_hex_string() {
        let raw = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
        let parsed: BuilderCode = serde_json::from_str(&format!("\"{}\"", raw)).unwrap();
        assert_eq!(parsed.as_hex(), raw);
    }

    #[test]
    fn builder_code_rejects_wrong_length() {
        let err = parse_builder_code("0xdeadbeef").unwrap_err();
        assert!(format!("{}", err).contains("32 bytes"));
    }

    #[test]
    fn builder_code_rejects_non_hex() {
        let err = parse_builder_code(
            "0xzz00000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(format!("{}", err).contains("non-hex"));
    }

    #[test]
    fn builder_code_rejects_non_ascii_without_panicking() {
        let raw = format!("0x0{}{}", "é", "0".repeat(61));
        let err = parse_builder_code(&raw).unwrap_err();
        assert!(format!("{}", err).contains("non-ASCII"));
    }

    #[test]
    fn clob_side_maps_buy_sell() {
        assert_eq!(clob_side(TradeDirection::Buy), 0);
        assert_eq!(clob_side(TradeDirection::Sell), 1);
    }
}
