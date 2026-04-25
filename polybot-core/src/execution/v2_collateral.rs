use alloy_sol_types::{sol, SolCall};
use polybot_common::errors::PolybotError;
use polybot_common::types::TransactionKind;
use polymarket_client_sdk::types::{Address, B256, U256};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::config::CollateralConfig;

pub const CONDITIONAL_TOKENS_ADDRESS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const STANDARD_EXCHANGE_ADDRESS: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
pub const NEG_RISK_EXCHANGE_ADDRESS: &str = "0xe2222d279d744050d28e00520010520000310F59";

const PUSD_DECIMALS: u64 = 1_000_000;

sol! {
    interface IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
    }

    interface IERC1155 {
        function setApprovalForAll(address operator, bool approved) external;
    }

    interface CollateralOnramp {
        function wrap(address _asset, address _to, uint256 _amount) external;
    }

    interface CollateralOfframp {
        function unwrap(address _asset, address _to, uint256 _amount) external;
    }

    interface ConditionalTokens {
        function redeemPositions(
            address collateralToken,
            bytes32 parentCollectionId,
            bytes32 conditionId,
            uint256[] indexSets
        ) external;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GaslessTransaction {
    pub to: String,
    pub data: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CollateralOperationPlan {
    pub kind: TransactionKind,
    pub description: String,
    pub transactions: Vec<GaslessTransaction>,
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    format!(
        "0x{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn parse_address(value: &str, field: &str) -> Result<Address, PolybotError> {
    value
        .parse()
        .map_err(|err| PolybotError::Config(format!("Invalid {field} address: {err}")))
}

fn parse_bytes32(value: &str, field: &str) -> Result<B256, PolybotError> {
    value
        .parse()
        .map_err(|err| PolybotError::Config(format!("Invalid {field} bytes32: {err}")))
}

fn amount_to_base_units(amount: Decimal) -> Result<U256, PolybotError> {
    if amount <= Decimal::ZERO {
        return Err(PolybotError::Config(
            "Collateral operation amount must be positive".to_string(),
        ));
    }

    let scaled = (amount * Decimal::from(PUSD_DECIMALS)).trunc();
    let base_units = scaled.to_u128().ok_or_else(|| {
        PolybotError::Config(format!(
            "Collateral operation amount {} is too large to encode",
            amount
        ))
    })?;
    if base_units == 0 {
        return Err(PolybotError::Config(
            "Collateral operation amount is below 0.000001 pUSD/USDC.e".to_string(),
        ));
    }

    Ok(U256::from(base_units))
}

fn tx(to: &str, data: Vec<u8>) -> GaslessTransaction {
    GaslessTransaction {
        to: to.to_string(),
        data: bytes_to_hex(&data),
        value: "0".to_string(),
    }
}

fn erc20_approve_tx(
    token: &str,
    spender: Address,
    amount: U256,
) -> Result<GaslessTransaction, PolybotError> {
    parse_address(token, "ERC20 token")?;
    Ok(tx(
        token,
        IERC20::approveCall { spender, amount }.abi_encode(),
    ))
}

fn erc1155_approve_all_tx(
    token: &str,
    operator: Address,
) -> Result<GaslessTransaction, PolybotError> {
    parse_address(token, "ERC1155 token")?;
    Ok(tx(
        token,
        IERC1155::setApprovalForAllCall {
            operator,
            approved: true,
        }
        .abi_encode(),
    ))
}

pub fn build_wrap_plan(
    config: &CollateralConfig,
    recipient: &str,
    amount: Decimal,
) -> Result<CollateralOperationPlan, PolybotError> {
    let recipient = parse_address(recipient, "wrap recipient")?;
    let usdc_e = parse_address(&config.usdc_e_address, "USDC.e")?;
    let onramp = parse_address(&config.onramp_address, "CollateralOnramp")?;
    let amount = amount_to_base_units(amount)?;

    Ok(CollateralOperationPlan {
        kind: TransactionKind::Wrap,
        description: "Approve USDC.e and wrap into pUSD".to_string(),
        transactions: vec![
            erc20_approve_tx(&config.usdc_e_address, onramp, amount)?,
            tx(
                &config.onramp_address,
                CollateralOnramp::wrapCall {
                    _asset: usdc_e,
                    _to: recipient,
                    _amount: amount,
                }
                .abi_encode(),
            ),
        ],
    })
}

pub fn build_unwrap_plan(
    config: &CollateralConfig,
    recipient: &str,
    amount: Decimal,
) -> Result<CollateralOperationPlan, PolybotError> {
    let recipient = parse_address(recipient, "unwrap recipient")?;
    let usdc_e = parse_address(&config.usdc_e_address, "USDC.e")?;
    let offramp = parse_address(&config.offramp_address, "CollateralOfframp")?;
    let amount = amount_to_base_units(amount)?;

    Ok(CollateralOperationPlan {
        kind: TransactionKind::Unwrap,
        description: "Approve pUSD and unwrap into USDC.e".to_string(),
        transactions: vec![
            erc20_approve_tx(&config.pusd_address, offramp, amount)?,
            tx(
                &config.offramp_address,
                CollateralOfframp::unwrapCall {
                    _asset: usdc_e,
                    _to: recipient,
                    _amount: amount,
                }
                .abi_encode(),
            ),
        ],
    })
}

pub fn build_trading_approval_plan(
    config: &CollateralConfig,
) -> Result<CollateralOperationPlan, PolybotError> {
    let ctf = parse_address(CONDITIONAL_TOKENS_ADDRESS, "ConditionalTokens")?;
    let standard_exchange = parse_address(STANDARD_EXCHANGE_ADDRESS, "CTF Exchange")?;
    let neg_risk_exchange = parse_address(NEG_RISK_EXCHANGE_ADDRESS, "Neg Risk CTF Exchange")?;

    Ok(CollateralOperationPlan {
        kind: TransactionKind::Approve,
        description: "Approve pUSD and CTF outcome tokens for CLOB V2 trading".to_string(),
        transactions: vec![
            erc20_approve_tx(&config.pusd_address, ctf, U256::MAX)?,
            erc1155_approve_all_tx(CONDITIONAL_TOKENS_ADDRESS, standard_exchange)?,
            erc1155_approve_all_tx(CONDITIONAL_TOKENS_ADDRESS, neg_risk_exchange)?,
        ],
    })
}

pub fn build_redeem_positions_plan(
    config: &CollateralConfig,
    condition_id: &str,
    index_sets: Vec<u64>,
) -> Result<CollateralOperationPlan, PolybotError> {
    if index_sets.is_empty() {
        return Err(PolybotError::Config(
            "Redeem positions requires at least one index set".to_string(),
        ));
    }

    let collateral_token = parse_address(&config.pusd_address, "pUSD")?;
    let parent_collection_id = B256::ZERO;
    let condition_id = parse_bytes32(condition_id, "condition_id")?;
    let index_sets = index_sets.into_iter().map(U256::from).collect();

    Ok(CollateralOperationPlan {
        kind: TransactionKind::Redeem,
        description: "Redeem resolved CTF positions into pUSD".to_string(),
        transactions: vec![tx(
            CONDITIONAL_TOKENS_ADDRESS,
            ConditionalTokens::redeemPositionsCall {
                collateralToken: collateral_token,
                parentCollectionId: parent_collection_id,
                conditionId: condition_id,
                indexSets: index_sets,
            }
            .abi_encode(),
        )],
    })
}

fn validate_plan_shape(plan: &CollateralOperationPlan) -> Result<usize, PolybotError> {
    if plan.transactions.is_empty() {
        return Err(PolybotError::Config(format!(
            "{:?} collateral operation produced no transactions",
            plan.kind
        )));
    }

    for transaction in &plan.transactions {
        parse_address(&transaction.to, "gasless transaction target")?;
        if !transaction.data.starts_with("0x") || transaction.data.len() < 10 {
            return Err(PolybotError::Config(format!(
                "{:?} collateral operation produced malformed calldata",
                plan.kind
            )));
        }
        if transaction.value != "0" {
            return Err(PolybotError::Config(format!(
                "{:?} collateral operation must not send POL value",
                plan.kind
            )));
        }
    }

    Ok(plan.transactions.len())
}

pub fn validate_live_collateral_operation_plans(
    config: &CollateralConfig,
    trading_wallet: &str,
    condition_id: &str,
) -> Result<usize, PolybotError> {
    let plans = [
        build_wrap_plan(config, trading_wallet, Decimal::ONE)?,
        build_unwrap_plan(config, trading_wallet, Decimal::ONE)?,
        build_trading_approval_plan(config)?,
        build_redeem_positions_plan(config, condition_id, vec![1, 2])?,
    ];

    plans.iter().try_fold(0usize, |count, plan| {
        validate_plan_shape(plan).map(|plan_count| count + plan_count)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn config() -> CollateralConfig {
        CollateralConfig {
            token: "pUSD".to_string(),
            pusd_address: "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB".to_string(),
            onramp_address: "0x93070a847efEf7F70739046A929D47a521F5B8ee".to_string(),
            offramp_address: "0x2957922Eb93258b93368531d39fAcCA3B4dC5854".to_string(),
            usdc_e_address: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
        }
    }

    #[test]
    fn amount_conversion_uses_six_decimals() {
        assert_eq!(
            amount_to_base_units(dec!(1.23)).unwrap(),
            U256::from(1_230_000u64)
        );
        assert_eq!(
            amount_to_base_units(dec!(12.3456789)).unwrap(),
            U256::from(12_345_678u64)
        );
    }

    #[test]
    fn amount_conversion_rejects_zero_or_too_small_amounts() {
        assert!(amount_to_base_units(dec!(0)).is_err());
        assert!(amount_to_base_units(dec!(0.0000009)).is_err());
    }

    #[test]
    fn wrap_plan_approves_usdce_then_calls_onramp() {
        let plan = build_wrap_plan(
            &config(),
            "0x1111111111111111111111111111111111111111",
            dec!(25),
        )
        .unwrap();

        assert_eq!(plan.kind, TransactionKind::Wrap);
        assert_eq!(plan.transactions.len(), 2);
        assert_eq!(plan.transactions[0].to, config().usdc_e_address);
        assert!(plan.transactions[0].data.starts_with("0x095ea7b3"));
        assert_eq!(plan.transactions[1].to, config().onramp_address);
        assert_eq!(plan.transactions[1].value, "0");
    }

    #[test]
    fn unwrap_plan_approves_pusd_then_calls_offramp() {
        let plan = build_unwrap_plan(
            &config(),
            "0x1111111111111111111111111111111111111111",
            dec!(10),
        )
        .unwrap();

        assert_eq!(plan.kind, TransactionKind::Unwrap);
        assert_eq!(plan.transactions.len(), 2);
        assert_eq!(plan.transactions[0].to, config().pusd_address);
        assert_eq!(plan.transactions[1].to, config().offramp_address);
    }

    #[test]
    fn trading_approval_plan_approves_pusd_and_outcome_tokens() {
        let plan = build_trading_approval_plan(&config()).unwrap();

        assert_eq!(plan.kind, TransactionKind::Approve);
        assert_eq!(plan.transactions.len(), 3);
        assert_eq!(plan.transactions[0].to, config().pusd_address);
        assert_eq!(plan.transactions[1].to, CONDITIONAL_TOKENS_ADDRESS);
        assert_eq!(plan.transactions[2].to, CONDITIONAL_TOKENS_ADDRESS);
    }

    #[test]
    fn redeem_plan_targets_conditional_tokens_contract() {
        let condition_id = "0xaf5e903876ad42de97e1cf02c2ef8484df69bcfc5541b96a400116557d1e504e";
        let plan = build_redeem_positions_plan(&config(), condition_id, vec![1, 2]).unwrap();

        assert_eq!(plan.kind, TransactionKind::Redeem);
        assert_eq!(plan.transactions.len(), 1);
        assert_eq!(plan.transactions[0].to, CONDITIONAL_TOKENS_ADDRESS);
        assert!(plan.transactions[0].data.starts_with("0x"));
    }

    #[test]
    fn redeem_plan_rejects_empty_index_sets() {
        let condition_id = "0xaf5e903876ad42de97e1cf02c2ef8484df69bcfc5541b96a400116557d1e504e";
        let err = build_redeem_positions_plan(&config(), condition_id, vec![]).unwrap_err();

        assert!(err.to_string().contains("at least one index set"));
    }

    #[test]
    fn live_collateral_plan_validation_covers_all_operation_types() {
        let condition_id = "0xaf5e903876ad42de97e1cf02c2ef8484df69bcfc5541b96a400116557d1e504e";
        let transaction_count = validate_live_collateral_operation_plans(
            &config(),
            "0x1111111111111111111111111111111111111111",
            condition_id,
        )
        .unwrap();

        assert_eq!(transaction_count, 8);
    }

    #[test]
    fn live_collateral_plan_validation_rejects_bad_condition_id() {
        let err = validate_live_collateral_operation_plans(
            &config(),
            "0x1111111111111111111111111111111111111111",
            "bad-condition-id",
        )
        .unwrap_err();

        assert!(err.to_string().contains("condition_id"));
    }
}
