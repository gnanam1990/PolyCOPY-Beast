use alloy_sol_types::{eip712_domain, sol, SolStruct};
use polybot_common::errors::PolybotError;
use polybot_common::types::TradeDirection;
use polymarket_client_sdk::auth::Signer;
use polymarket_client_sdk::types::{Address, B256, U256};
use rust_decimal::prelude::ToPrimitive;

use super::order_builder::Order;

pub const V2_EIP712_DOMAIN_NAME: &str = "Polymarket CTF Exchange";
pub const V2_EIP712_DOMAIN_VERSION: &str = "2";
pub const V2_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

const FIXED_DECIMALS: u32 = 6;

sol! {
    struct V2OrderStruct {
        uint256 salt;
        address maker;
        address signer;
        address taker;
        uint256 tokenId;
        uint256 makerAmount;
        uint256 takerAmount;
        uint256 expiration;
        uint256 nonce;
        uint256 feeRateBps;
        uint8 side;
        uint8 signatureType;
        uint256 timestamp;
        bytes32 metadata;
        bytes32 builder;
    }
}

pub fn parse_builder_code(raw: &str) -> Result<[u8; 32], PolybotError> {
    let stripped = raw.strip_prefix("0x").unwrap_or(raw);
    if stripped.len() != 64 {
        return Err(PolybotError::Execution(format!(
            "Invalid BUILDER_CODE length: expected 32-byte hex, got {} chars",
            stripped.len()
        )));
    }

    let mut out = [0u8; 32];
    for (idx, chunk) in stripped.as_bytes().chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(chunk)
            .map_err(|e| PolybotError::Execution(format!("Invalid BUILDER_CODE UTF-8: {}", e)))?;
        out[idx] = u8::from_str_radix(hex, 16)
            .map_err(|e| PolybotError::Execution(format!("Invalid BUILDER_CODE hex: {}", e)))?;
    }

    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2OrderPayload {
    pub salt: u64,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub side: u8,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: u64,
    pub nonce: u64,
    pub fee_rate_bps: u64,
    pub signature_type: u8,
    pub signature: String,
    pub timestamp_ms: u64,
    pub builder_code: [u8; 32],
    pub metadata: String,
}

fn to_fixed_u128(d: rust_decimal::Decimal) -> u128 {
    d.normalize()
        .trunc_with_scale(FIXED_DECIMALS)
        .mantissa()
        .to_u128()
        .expect("positive fixed-point amount should fit into u128")
}

fn generate_salt(timestamp_ms: u64) -> u64 {
    timestamp_ms & ((1 << 53) - 1)
}

fn bytes32_hex(bytes: &[u8; 32]) -> String {
    format!(
        "0x{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

pub fn build_v2_order_payload(
    order: &Order,
    maker_address: &str,
    signer_address: &str,
    builder_code: &str,
    timestamp_ms: u64,
) -> Result<V2OrderPayload, PolybotError> {
    let builder_code = parse_builder_code(builder_code)?;
    let side = match order.direction {
        TradeDirection::Buy => 0,
        TradeDirection::Sell => 1,
    };
    let maker_amount_raw = match order.direction {
        TradeDirection::Buy => order.size_usd,
        TradeDirection::Sell => order.size,
    };
    let taker_amount_raw = match order.direction {
        TradeDirection::Buy => order.size,
        TradeDirection::Sell => order.size_usd,
    };
    let maker_amount = U256::from(to_fixed_u128(maker_amount_raw)).to_string();
    let taker_amount = U256::from(to_fixed_u128(taker_amount_raw)).to_string();

    Ok(V2OrderPayload {
        salt: generate_salt(timestamp_ms),
        maker: maker_address.to_string(),
        signer: signer_address.to_string(),
        taker: "0x0000000000000000000000000000000000000000".to_string(),
        token_id: order.token_id.clone(),
        side,
        maker_amount,
        taker_amount,
        expiration: 0,
        nonce: 0,
        fee_rate_bps: 0,
        signature_type: 0,
        signature: String::new(),
        timestamp_ms,
        builder_code,
        metadata: bytes32_hex(&[0u8; 32]),
    })
}

pub async fn sign_v2_order_payload<S: Signer>(
    signer: &S,
    payload: &V2OrderPayload,
) -> Result<String, PolybotError> {
    let chain_id = signer.chain_id().ok_or_else(|| {
        PolybotError::Execution("Missing signer chain id for V2 signing".to_string())
    })?;
    let maker: Address = payload
        .maker
        .parse()
        .map_err(|e| PolybotError::Execution(format!("Invalid maker address: {}", e)))?;
    let signer_addr: Address = payload
        .signer
        .parse()
        .map_err(|e| PolybotError::Execution(format!("Invalid signer address: {}", e)))?;
    let taker: Address = payload
        .taker
        .parse()
        .map_err(|e| PolybotError::Execution(format!("Invalid taker address: {}", e)))?;
    let token_id = U256::from_str_radix(&payload.token_id, 10)
        .map_err(|e| PolybotError::Execution(format!("Invalid token id for V2 signing: {}", e)))?;
    let maker_amount = U256::from_str_radix(&payload.maker_amount, 10)
        .map_err(|e| PolybotError::Execution(format!("Invalid makerAmount: {}", e)))?;
    let taker_amount = U256::from_str_radix(&payload.taker_amount, 10)
        .map_err(|e| PolybotError::Execution(format!("Invalid takerAmount: {}", e)))?;
    let metadata: B256 = payload
        .metadata
        .parse()
        .map_err(|e| PolybotError::Execution(format!("Invalid metadata bytes32: {}", e)))?;
    let builder = B256::from(payload.builder_code);
    let verifying_contract: Address = V2_STANDARD_EXCHANGE
        .parse()
        .map_err(|e| PolybotError::Execution(format!("Invalid V2 exchange address: {}", e)))?;

    let order = V2OrderStruct {
        salt: U256::from(payload.salt),
        maker,
        signer: signer_addr,
        taker,
        tokenId: token_id,
        makerAmount: maker_amount,
        takerAmount: taker_amount,
        expiration: U256::from(payload.expiration),
        nonce: U256::from(payload.nonce),
        feeRateBps: U256::from(payload.fee_rate_bps),
        side: payload.side,
        signatureType: payload.signature_type,
        timestamp: U256::from(payload.timestamp_ms),
        metadata,
        builder,
    };

    let domain = eip712_domain!(
        name: V2_EIP712_DOMAIN_NAME,
        version: V2_EIP712_DOMAIN_VERSION,
        chain_id: chain_id,
        verifying_contract: verifying_contract,
    );

    let signature = signer
        .sign_hash(&order.eip712_signing_hash(&domain))
        .await
        .map_err(|e| PolybotError::Execution(format!("Failed to sign V2 order: {}", e)))?;

    Ok(signature.to_string())
}

pub fn payload_to_relayer_json(payload: &V2OrderPayload) -> serde_json::Value {
    serde_json::json!({
        "salt": payload.salt,
        "maker": payload.maker,
        "signer": payload.signer,
        "taker": payload.taker,
        "tokenId": payload.token_id,
        "makerAmount": payload.maker_amount,
        "takerAmount": payload.taker_amount,
        "expiration": payload.expiration,
        "nonce": payload.nonce,
        "feeRateBps": payload.fee_rate_bps,
        "side": payload.side,
        "signatureType": payload.signature_type,
        "signature": payload.signature,
        "timestamp": payload.timestamp_ms,
        "metadata": payload.metadata,
        "builder": bytes32_hex(&payload.builder_code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn builder_code_accepts_bytes32_hex() {
        let code = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
        let parsed = parse_builder_code(code).unwrap();
        assert_eq!(parsed.len(), 32);
        assert_eq!(parsed[28], 0xde);
        assert_eq!(parsed[29], 0xad);
        assert_eq!(parsed[30], 0xbe);
        assert_eq!(parsed[31], 0xef);
    }

    #[test]
    fn builder_code_rejects_wrong_length() {
        let err = parse_builder_code("0xdeadbeef").unwrap_err();
        match err {
            PolybotError::Execution(message) => assert!(message.contains("length")),
            other => panic!("expected execution error, got {other:?}"),
        }
    }

    #[test]
    fn v2_domain_version_is_fixed() {
        assert_eq!(V2_EIP712_DOMAIN_NAME, "Polymarket CTF Exchange");
        assert_eq!(V2_EIP712_DOMAIN_VERSION, "2");
    }

    #[test]
    fn build_v2_order_payload_sets_v2_specific_fields() {
        let order = Order {
            signal_id: "sig-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            token_id: "123456789".to_string(),
            category: polybot_common::types::Category::Politics,
            side: polybot_common::types::Side::Yes,
            direction: TradeDirection::Buy,
            price: rust_decimal_macros::dec!(0.50),
            size: rust_decimal_macros::dec!(10),
            size_usd: rust_decimal_macros::dec!(5),
            order_type: polybot_common::types::OrderType::Limit,
        };

        let payload = build_v2_order_payload(
            &order,
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000002",
            "0x00000000000000000000000000000000000000000000000000000000deadbeef",
            1_712_000_000_000,
        )
        .unwrap();

        assert_eq!(payload.side, 0);
        assert_eq!(payload.fee_rate_bps, 0);
        assert_eq!(payload.signature_type, 0);
        assert_eq!(payload.timestamp_ms, 1_712_000_000_000);
        assert_eq!(payload.maker_amount, U256::from(5_000_000u64).to_string());
        assert_eq!(payload.taker_amount, U256::from(10_000_000u64).to_string());
    }

    #[tokio::test]
    async fn sign_v2_order_payload_returns_prefixed_signature() {
        let signer = polymarket_client_sdk::auth::LocalSigner::from_str(
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap()
        .with_chain_id(Some(137));
        let order = Order {
            signal_id: "sig-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            token_id: "123456789".to_string(),
            category: polybot_common::types::Category::Politics,
            side: polybot_common::types::Side::Yes,
            direction: TradeDirection::Buy,
            price: rust_decimal_macros::dec!(0.50),
            size: rust_decimal_macros::dec!(10),
            size_usd: rust_decimal_macros::dec!(5),
            order_type: polybot_common::types::OrderType::Limit,
        };
        let payload = build_v2_order_payload(
            &order,
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000001",
            "0x00000000000000000000000000000000000000000000000000000000deadbeef",
            1_712_000_000_000,
        )
        .unwrap();

        let signature = sign_v2_order_payload(&signer, &payload).await.unwrap();

        assert!(signature.starts_with("0x"));
        assert!(signature.len() > 10);
    }
}
