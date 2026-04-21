use polybot_common::types::ExecutionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportPlan {
    pub uses_market_data: bool,
    pub uses_ws_market_data: bool,
    pub submits_orders: bool,
}

pub fn select_transport_mode(mode: ExecutionMode) -> TransportPlan {
    TransportPlan {
        uses_market_data: mode.allows_network_market_data(),
        uses_ws_market_data: mode.allows_ws_market_data(),
        submits_orders: mode.allows_live_order_submission(),
    }
}
