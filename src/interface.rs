use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

pub type FtxId = u64;
pub type FtxPrice = f32;
pub type FtxSize = f32;
pub type PriceLadder = Vec<(FtxPrice, FtxSize)>;



#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FuturePosition {
    pub cost: FtxPrice,
    pub entry_price: Option<FtxPrice>,
    pub future: String,
    pub initial_margin_requirement: FtxPrice,
    pub long_order_size: FtxSize,
    pub maintenance_margin_requirement: FtxPrice, // Unclear if this should be FtxVolume
    /// Size of position. Positive if long, negative if short.
    pub net_size: FtxPrice,
    pub open_size: FtxPrice,
    pub realized_pnl: FtxPrice,
    pub unrealized_pnl: FtxPrice,
    pub short_order_size: FtxSize,
    pub side: String,
    /// Absolute value of net_size
    pub size: FtxPrice,
}


#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub backstop_provider: bool,
    pub collateral: FtxPrice,
    pub free_collateral: FtxPrice,
    pub initial_margin_requirement: FtxPrice,
    pub liquidating: bool,
    pub maintenance_margin_requirement: FtxPrice,
    pub maker_fee: FtxPrice,
    pub margin_fraction: Option<FtxPrice>,
    pub open_margin_fraction: Option<FtxPrice>,
    pub taker_fee: FtxPrice,
    pub total_account_value: FtxPrice,
    pub total_position_size: FtxSize,
    pub username: String,
    pub leverage: FtxPrice,
    pub positions: Vec<FuturePosition>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfoResponse {
    pub success: bool,
    pub result: Option<AccountInfo>,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    pub name: String,
    pub enabled: bool,
    pub price_increment: FtxPrice,
    pub size_increment: FtxSize,
    #[serde(rename = "type")]
    pub market_type: String,
    pub base_currency: Option<String>,
    pub quote_currency: Option<String>,
    pub underlying: Option<String>,
    pub restricted: bool,
    pub future: Option<FutureInstrument>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FutureInstrument {
    pub name: String,
    pub underlying: String,
    pub description: String,
    #[serde(rename = "type")]
    pub future_type: String,
    pub expiry: Option<String>,
    pub perpetual: bool,
    pub expired: bool,
    pub enabled: bool,
    pub post_only: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalances {
    pub success: bool,
    pub result: Vec<BalanceEntry>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestResponseOrderList {
    pub success: bool,
    pub result: Vec<LimitOrder>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestResponseMarketList {
    pub success: bool,
    pub result: Vec<Market>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LimitOrder {
    pub id: FtxId,
    pub client_id: Option<String>,
    pub market: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub side: String,
    pub size: FtxSize,
    pub price: FtxPrice,
    pub reduce_only: bool,
    pub ioc: bool,
    pub post_only: bool,
    pub status: String,
    pub filled_size: FtxSize,
    pub remaining_size: FtxSize,
    pub avg_fill_price: Option<FtxPrice>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Markets {
    pub data: FxHashMap<String, Market>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FtxLoginSignature {
    pub key: String,
    pub sign: String,
    pub time: u128,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FtxLogin {
    pub args: FtxLoginSignature,
    pub op: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "channel", content = "data")]
pub enum PartialData {
    Markets(Markets),
    Orderbook(OrderBookUpdate),
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ticker {
    pub bid: Option<FtxPrice>,
    pub ask: Option<FtxPrice>,
    pub bid_size: FtxSize,
    pub ask_size: FtxSize,
    pub last: Option<FtxPrice>,
    pub time: f64,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Trades<'a> {
    pub id: FtxId,
    pub price: FtxPrice,
    pub size: FtxSize,
    pub side: &'a str,
    pub liquidation: bool,
    pub time: &'a str,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "channel", content = "data")]
pub enum UpdateData<'a> {
    Ticker(Ticker),
    Orderbook(OrderBookUpdate),
    #[serde(borrow)]
    Trades(Vec<Trades<'a>>),
    Orders(LimitOrder),
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum FtxMessage<'a> {
    Pong {},
    Subscribed {
        channel: String,
    },
    Partial {
        market: Option<String>,
        #[serde(flatten)]
        data: PartialData,
    },
    Update {
        #[serde(flatten)]
        #[serde(borrow)]
        data: UpdateData<'a>,
        market : String
    },
    Error {
        code: i32,
        msg: String,
    },
    Info {
        code: i32,
        msg: String,
    },
    Unsubscribed {
        channel: String,
        market: String,
    },
}


#[derive(Deserialize, Debug, Clone)]
pub struct OrderBookUpdate {
    pub time: f64,
    pub checksum: u32,
    pub bids: PriceLadder,
    pub asks: PriceLadder,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrder<'a> {
    pub market: &'a str,
    pub side: &'a str,
    pub price: Option<FtxPrice>,
    #[serde(rename = "type")]
    pub order_type: &'a str,
    pub size: FtxSize,
    pub reduce_only: bool,
    pub ioc: bool,
    pub post_only: bool,
    pub client_id: Option<&'a str>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BalanceEntry {
    pub coin: String,
    pub free: FtxPrice,
    pub spot_borrow: FtxPrice,
    pub total: FtxPrice,
    pub usd_value: FtxPrice,
    pub available_without_borrow: FtxPrice,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderStatus {
    pub created_at: String,
    pub filled_size: FtxSize,
    pub future: Option<String>,
    pub id: u64,
    pub market: String,
    pub price: FtxPrice,
    pub remaining_size: FtxSize,
    pub side: String,
    pub size: FtxSize,
    pub status: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub reduce_only: bool,
    pub ioc: bool,
    pub post_only: bool,
    pub client_id: Option<String>,
}


#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrderResponse {
    pub success: bool,
    pub result: Option<OrderStatus>,
    pub error: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    pub success: bool,
    pub result: String,
}

