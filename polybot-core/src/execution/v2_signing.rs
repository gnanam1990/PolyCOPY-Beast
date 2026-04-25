use polybot_common::errors::PolybotError;
use serde::{Deserialize, Serialize};

use super::v2_order::V2OrderPayload;

pub const EIP712_DOMAIN_NAME: &str = "Polymarket CTF Exchange";
pub const EIP712_DOMAIN_VERSION: &str = "2";
pub const POLYGON_CHAIN_ID: u64 = 137;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eip712DomainV2 {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: String,
}

pub fn exchange_domain(verifying_contract: &str) -> Result<Eip712DomainV2, PolybotError> {
    validate_address(verifying_contract)?;
    Ok(Eip712DomainV2 {
        name: EIP712_DOMAIN_NAME.to_string(),
        version: EIP712_DOMAIN_VERSION.to_string(),
        chain_id: POLYGON_CHAIN_ID,
        verifying_contract: verifying_contract.to_string(),
    })
}

pub fn validate_address(address: &str) -> Result<(), PolybotError> {
    let valid = address.starts_with("0x")
        && address.len() == 42
        && address[2..].chars().all(|ch| ch.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(PolybotError::Config(format!(
            "invalid EIP-712 verifying contract address: {}",
            address
        )))
    }
}

pub fn signing_payload_json(
    domain: &Eip712DomainV2,
    order: &V2OrderPayload,
) -> serde_json::Value {
    serde_json::json!({
        "domain": domain,
        "primaryType": "Order",
        "message": {
            "salt": order.salt,
            "maker": order.maker,
            "signer": order.signer,
            "tokenId": order.token_id,
            "makerAmount": order.maker_amount,
            "takerAmount": order.taker_amount,
            "side": order.side,
            "signatureType": order.signature_type,
            "timestamp": order.timestamp_ms,
            "metadata": order.metadata.as_hex(),
            "builder": order.builder.as_hex(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::v2_order::{parse_builder_code, V2OrderPayload};
    use polybot_common::types::OrderType;

    #[test]
    fn eip712_domain_uses_version_2() {
        let domain = exchange_domain("0xE111180000d2663C0091e4f400237545B87B996B").unwrap();
        assert_eq!(domain.name, "Polymarket CTF Exchange");
        assert_eq!(domain.version, "2");
        assert_eq!(domain.chain_id, 137);
    }

    #[test]
    fn invalid_verifying_contract_is_rejected() {
        let err = exchange_domain("0xabc").unwrap_err();
        assert!(format!("{}", err).contains("invalid EIP-712"));
    }

    #[test]
    fn signing_payload_contains_builder_and_timestamp() {
        let builder =
            parse_builder_code("0x00000000000000000000000000000000000000000000000000000000deadbeef")
                .unwrap();
        let order = V2OrderPayload {
            salt: 42,
            maker: "0x0000000000000000000000000000000000000001".to_string(),
            signer: "0x0000000000000000000000000000000000000001".to_string(),
            token_id: "123".to_string(),
            maker_amount: "1000000".to_string(),
            taker_amount: "1000000".to_string(),
            side: 0,
            signature_type: 0,
            timestamp_ms: 1_771_000_000_000,
            metadata: builder,
            builder,
            order_type: OrderType::Limit,
            fee_schedule: None,
        };
        let domain = exchange_domain("0xE111180000d2663C0091e4f400237545B87B996B").unwrap();
        let payload = signing_payload_json(&domain, &order);
        assert_eq!(payload["primaryType"], "Order");
        assert_eq!(payload["message"]["timestamp"], 1_771_000_000_000u64);
        assert_eq!(
            payload["message"]["builder"],
            "0x00000000000000000000000000000000000000000000000000000000deadbeef"
        );
    }
}
