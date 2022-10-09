use const_format::concatcp;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use hmac::{Hmac, Mac};
use num_traits::Zero;
use reqwest::Response;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Mutex};
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

const FTX_WEBSOCKET_URL: &str = "wss://ftx.com/ws/";
const FTX_REST_URL: &str = "https://ftx.com";

const PING_MSG: &str = r#"{"op":"ping"}"#;

const URI_GET_ACCOUNT_INFO: &str = "/api/account";
const URL_GET_ACCOUNT_INFO: &str = concatcp!(FTX_REST_URL, URI_GET_ACCOUNT_INFO);
const URI_GET_WALLET: &str = "/api/wallet/balances";
const URL_GET_WALLET: &str = concatcp!(FTX_REST_URL, URI_GET_WALLET);
const URI_ORDERS: &str = "/api/orders";
const URL_ORDERS: &str = concatcp!(FTX_REST_URL, URI_ORDERS);
const URI_MARKETS: &str = "/api/markets";
const URL_MARKETS: &str = concatcp!(FTX_REST_URL, URI_MARKETS);

pub type FtxOrderId = u64;
pub type FtxPrice = f32;
pub type FtxSize = f32;

type WsWriter = SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Message,
>;

type WsReader = SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum SideOfBook {
    BUY = 0,
    SELL = 1,
}

#[derive(Debug, Clone)]
pub enum UpdateMessage {
    OrderbookSnapshot(OrderBookUpdate),
    OrderbookUpdate(OrderBookUpdate),
    OrderFilled {
        id: FtxOrderId,
        side: SideOfBook,
        fill_size: FtxSize,
    },
    OrderCancelled {
        id: FtxOrderId,
        side: SideOfBook,
    },
    BestPrice {
        bid: Option<FtxPrice>,
        ask: Option<FtxPrice>,
        bid_size: FtxSize,
        ask_size: FtxSize,
        last_trade: Option<FtxPrice>,
    },
    Trade {
        price : FtxPrice,
        size : FtxSize,
        timestamp : i64,
        side : SideOfBook
    }
}


#[derive(Debug, Clone)]
pub enum FailureReason {
    NetworkError,
    InsufficientFunds,
    OrderCancelled,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct FtxLoginSignature {
    key: String,
    sign: String,
    time: u128,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct FtxLogin {
    args: FtxLoginSignature,
    op: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct FtxFuture {
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

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FtxMarket {
    name: String,
    enabled: bool,
    price_increment: FtxPrice,
    size_increment: FtxSize,
    #[serde(rename = "type")]
    market_type: String,
    base_currency: Option<String>,
    quote_currency: Option<String>,
    underlying: Option<String>,
    restricted: bool,
    future: Option<FtxFuture>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LimitOrder {
    id: u64,
    client_id: Option<String>,
    market: String,
    #[serde(rename = "type")]
    order_type: String,
    side: String,
    size: FtxSize,
    price: FtxPrice,
    reduce_only: bool,
    ioc: bool,
    post_only: bool,
    status: String,
    filled_size: FtxSize,
    remaining_size: FtxSize,
    avg_fill_price: Option<FtxPrice>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Markets {
    pub data: FxHashMap<String, FtxMarket>,
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

type FtxPriceLadder = Vec<(FtxPrice, FtxSize)>;

// fn to_lob_update(prices: &FtxPriceLadder) -> lob::OrderBookUpdates {
//     let mut updates = lob::OrderBookUpdates::with_capacity(prices.len());
//     for p in prices.iter() {
//         updates.push((lob::to_price(p.0), p.1))
//     }
//     updates
// }

#[derive(Deserialize, Debug, Clone)]
pub struct OrderBookUpdate {
    pub time: f64,
    pub checksum: u32,
    pub bids: FtxPriceLadder,
    pub asks: FtxPriceLadder,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Trades<'a> {
    id: u32,
    price: FtxPrice,
    size: FtxSize,
    side: &'a str,
    liquidation: bool,
    time: String,
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

const CMD_SUBSCRIBE: &str = "subscribe";
const CMD_UNSUBSCRIBE: &str = "unsubscribe";

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SubscriptionMgmt<'a> {
    channel: &'a str,
    market: &'a str,
    op: &'a str,
}

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
    /// Absolute value of  net_size
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

impl AccountInfo {
    pub fn get_position_by_name(&self, future_name: &str) -> Option<&FuturePosition> {
        self.positions.iter().find(|&x| x.future == future_name)
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfoResponse {
    pub success: bool,
    pub result: Option<AccountInfo>,
    pub error: Option<String>,
    pub error_code: Option<String>,
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
    pub result: Vec<FtxMarket>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrder<'a> {
    market: &'a str,
    side: &'a str,
    price: FtxPrice,
    #[serde(rename = "type")]
    order_type: &'a str,
    size: FtxSize,
    reduce_only: bool,
    ioc: bool,
    post_only: bool,
    client_id: Option<&'a str>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderStatus {
    created_at: String,
    filled_size: FtxSize,
    future: Option<String>,
    id: u64,
    market: String,
    price: FtxPrice,
    remaining_size: FtxSize,
    side: String,
    size: FtxSize,
    status: String,
    #[serde(rename = "type")]
    order_type: String,
    reduce_only: bool,
    ioc: bool,
    post_only: bool,
    client_id: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrderResponse {
    success: bool,
    result: Option<OrderStatus>,
    error: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    success: bool,
    result: String,
}

/// Returns a signature for the authentication payload to enable private websocket channels.
fn build_ws_signature(timestamp: &str, api_secret: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(api_secret.as_bytes()).unwrap();
    let mut s = String::with_capacity(timestamp.len() + 20); // plenty big enough to avoid a realloc on the subsequent push_str's
    s.push_str(timestamp);
    s.push_str("websocket_login");
    mac.update(&s.into_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Returns a message signature for the provided payload to enable authenticated GET/POST requests to FTX.
fn build_signature(
    api_secret: &str,
    time_stamp: &str,
    http_cmd: &str,
    uri: &str,
    data: Option<&str>,
) -> String {
    const S_SIZE: usize = 256;
    let mut mac = Hmac::<Sha256>::new_from_slice(api_secret.as_bytes()).unwrap();
    let mut s = String::with_capacity(S_SIZE); // big enough to avoid a realloc on the subsequent push_str's
    s.push_str(time_stamp);
    s.push_str(http_cmd);
    s.push_str(uri);
    if data.is_some() {
        s.push_str(data.unwrap())
    };
    if cfg!(debug_assertions) {
        if s.len() > S_SIZE {
            warn!(
                "build_signature() string buffer too small ({:?} vs {:?})",
                s.len(),
                S_SIZE
            )
        };
    }
    mac.update(&s.into_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Returns the current time as an Unix EPOX timestamp in milliseconds and as a string.
fn get_timestamp() -> (u128, String) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    (ts, ts.to_string())
}

async fn start_websocket(ws_url: &str) -> (WsWriter, WsReader) {
    let url = url::Url::parse(ws_url).unwrap();
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (ws_writer, ws_reader) = ws_stream.split();
    (ws_writer, ws_reader)
}

async fn authenticate(
    ws_controller: &Arc<Mutex<WebSocketController>>,
    writer: &mut WsWriter,
    api_key : &str,
    api_secret : &str
) -> Result<(), ()> {
    ws_controller
        .lock()
        .await
        .authenticate(writer, api_key, api_secret)
        .await
        .map_err(|e| {
            error!("Authentication error: {:?}", e);
            ()
        })
}

#[inline]
fn send_lob_update<'a>(channel: &broadcast::Sender<UpdateMessage>, msg: UpdateMessage) {
    if channel.receiver_count() > 0 {
        if let Err(e) = channel.send(msg) {
            error!("lob channel send error: {}", e);
        };
    }
}

/// Worker method that receives JSON data from FTX and parses it.  
async fn ftx_data_worker(
    broadcast_channel: broadcast::Sender<UpdateMessage>,
    ws_controller: Arc<Mutex<WebSocketController>>,
    ws_url: &str,
    api_key : &str,
    api_secret : &str
) {
    info!("Ftx data worker started.");
    let mut ping_interval = time::interval(time::Duration::from_secs(15));
    let mut init_websocket = false;
    let (mut ws_writer, mut ws_reader) = start_websocket(ws_url).await;
    let _ = authenticate(&ws_controller, &mut ws_writer, api_key, api_secret).await;
    while !ws_controller.lock().await.should_terminate {
        if init_websocket {
            info!("Reinitializing web socket");
            (ws_writer, ws_reader) = start_websocket(ws_url).await;
            if authenticate(&ws_controller, &mut ws_writer, api_key, api_secret).await.is_ok() {
                ws_controller.lock().await.resubscribe(&mut ws_writer).await;
                init_websocket = false;
            }
        }
        ws_controller
            .lock()
            .await
            .check_for_changes(&mut ws_writer)
            .await;

        tokio::select! {
            _ = ping_interval.tick() => {
                let ping_msg = Message::from(PING_MSG);
                if let Err(e) = ws_writer.send(ping_msg).await {
                    warn!("Ping send failed: {:?}", e);
                };
            }
            Some(msg) = ws_reader.next() => {
                match msg {
                    Ok(msg) => {
                        let msg = msg.into_text().unwrap();
                        let msg: FtxMessage = serde_json::from_str(&msg).unwrap();
                        match msg {
                            FtxMessage::Partial { market: _, data } => match data {
                                PartialData::Orderbook(o) => {
                                    let payload = UpdateMessage::OrderbookSnapshot(o);
                                    send_lob_update(&broadcast_channel, payload);
                                }
                                PartialData::Markets(_) => {}
                            },
                            FtxMessage::Update { data } => match data {
                                UpdateData::Orderbook(o) => {
                                    let payload = UpdateMessage::OrderbookSnapshot(o);
                                    send_lob_update(&broadcast_channel, payload);
                                }
                                UpdateData::Ticker(t) => {
                                    let payload = UpdateMessage::BestPrice {
                                        bid: t.bid,
                                        ask: t.ask,
                                        bid_size: t.bid_size,
                                        ask_size: t.ask_size,
                                        last_trade: t.last,
                                    };
                                    send_lob_update(&broadcast_channel, payload);
                                }
                                UpdateData::Trades(_t) => {}
                                UpdateData::Orders(o) => {
                                    if o.status.eq("closed") {
                                        let payload = if o.filled_size.is_zero() {
                                            UpdateMessage::OrderCancelled {
                                                id: o.id.try_into().unwrap(),
                                                side: if o.side == "buy" {SideOfBook::BUY} else {SideOfBook::SELL}
                                            }
                                        } else {
                                            UpdateMessage::OrderFilled {
                                                id: o.id.try_into().unwrap(),
                                                side: if o.side == "buy" {SideOfBook::BUY} else {SideOfBook::SELL},
                                                fill_size: o.filled_size
                                            }
                                        };
                                        send_lob_update(&broadcast_channel, payload);
                                    };
                                }
                            },
                            FtxMessage::Error {code, msg } => {
                                error!("FTX Error on websocket {:?}:{:?}", code, msg);
                            },
                            FtxMessage::Info {code, msg} => {
                                info!("FTX Info on websocket {:?}:{:?}", code, msg);
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        error!("FTX data socket error: {}", e);
                        init_websocket = true;
                    }
                }
            }
        }
    }
    info!("Ftx data worker terminated.");
}

pub struct RestApi {
    client: reqwest::Client,
    api_key: String,
    api_secret : String
}

impl RestApi {
    pub fn new(api_key : &str, api_secret : &str) -> RestApi {
        Self {
            client: reqwest::Client::builder()
                .tcp_nodelay(true)
                .build()
                .unwrap(),
            api_key: String::from(api_key),
            api_secret: String::from(api_secret)
        }
    }

    /// Build and send a FTX GET request and returns a future for the response.
    fn send_get_request(
        &self,
        target_url: &str,
        endpoint: &str,
    ) -> impl std::future::Future<Output = Result<Response, reqwest::Error>> {
        let (_, ts) = get_timestamp();
        let signature = build_signature(&self.api_secret, &ts, "GET", endpoint, None);
        self.client
            .get(target_url)
            .header("FTX-KEY", &self.api_key)
            .header("FTX-SIGN", signature)
            .header("FTX-TS", ts)
            .send()
    }

    /// Returns all positions in futures contracts in the account wallet.
    pub async fn get_account_info(&self) -> Result<AccountInfo, ()> {
        let res = self
            .send_get_request(URL_GET_ACCOUNT_INFO, URI_GET_ACCOUNT_INFO)
            .await;

        let mut result: Result<AccountInfo, ()> = Err(());
        if let Ok(r) = res {
            let msg = r.text().await.unwrap();
            let msg: AccountInfoResponse = serde_json::from_str(&msg[..]).unwrap();
            result = msg.result.ok_or(());
            if !msg.success {
                warn!("get_account_info: {}", msg.error.unwrap())
            }
        }
        result
    }

    /// Returns all balances in the account wallet.
    pub async fn get_wallet(&self) -> Result<WalletBalances, ()> {
        let res = self.send_get_request(URL_GET_WALLET, URI_GET_WALLET).await;

        if let Ok(r) = res {
            let msg = r.text().await.unwrap();
            let msg = serde_json::from_str(&msg[..]).unwrap();
            return Ok(msg);
        }
        Err(())
    }

    /// Returns a list of all markets on the exchange.
    pub async fn get_markets(&self) -> Result<Vec<FtxMarket>, ()> {
        let res = self.send_get_request(URL_MARKETS, URI_MARKETS).await;

        if let Ok(r) = res {
            let msg = r.text().await.unwrap();
            let msg: RestResponseMarketList = serde_json::from_str(&msg[..]).unwrap();
            if msg.success {
                return Ok(msg.result);
            }
        }
        Err(())
    }
    /// Returns the list of active orders on the exchange for the current account.
    pub async fn get_orders(&self) -> Result<RestResponseOrderList, ()> {
        let res = self.send_get_request(URL_ORDERS, URI_ORDERS).await;

        if let Ok(r) = res {
            let msg = r.text().await.unwrap();
            let msg: RestResponseOrderList = serde_json::from_str(&msg[..]).unwrap();
            if msg.success {
                return Ok(msg);
            }
        }
        Err(())
    }

    pub async fn limit_order(
        &self,
        market: &str,
        side: &str,
        price: FtxPrice,
        order_type: &str,
        size: FtxSize,
        reduce_only: bool,
        ioc: bool,
        post_only: bool,
        client_id: Option<&str>,
    ) -> Result<FtxOrderId, String> {
        let body = PlaceOrder {
            market,
            side,
            price: price.floor(),
            order_type,
            size,
            reduce_only,
            ioc,
            post_only,
            client_id,
        };
        let endpoint = URI_ORDERS;
        let target_url = URL_ORDERS;
        let payload = serde_json::to_string(&body).unwrap();
        let (_, ts) = get_timestamp();
        let signature = build_signature(&self.api_key, &ts, "POST", endpoint, Some(&payload));

        let res = self
            .client
            .post(target_url)
            .header("FTX-KEY", &self.api_key)
            .header("FTX-SIGN", signature)
            .header("FTX-TS", ts)
            .body(payload)
            .send()
            .await;
        if let Ok(r) = res {
            let msg = r.text().await.unwrap();
            let msg: Result<PlaceOrderResponse, _> = serde_json::from_str(&msg[..]);
            if let Ok(m) = msg {
                if m.success {
                    let id = m.result.unwrap().id;
                    return Ok(id);
                } else {
                    return Err(m.error.unwrap());
                }
            } else {
                return Err(String::from("Serde error"));
            }
        }
        return Err(String::from("Unknown place_order error"));
    }

    pub async fn cancel_order(&self, order_id: FtxOrderId) -> Result<(), ()> {
        let mut endpoint = String::with_capacity(URI_ORDERS.len() + 20);
        endpoint.push_str(URI_ORDERS);
        endpoint.push_str("/");
        endpoint.push_str(&order_id.to_string());
        let mut target_url = String::with_capacity(URL_ORDERS.len() + 20);
        target_url.push_str(URL_ORDERS);
        target_url.push_str("/");
        target_url.push_str(&order_id.to_string());
        let (_, ts) = get_timestamp();
        let signature = build_signature(&self.api_secret, &ts, "DELETE", &endpoint, None);

        let res = self
            .client
            .delete(target_url)
            .header("FTX-KEY", &self.api_key)
            .header("FTX-SIGN", signature)
            .header("FTX-TS", ts)
            .send()
            .await;

        if let Ok(r) = res {
            let msg = r.text().await.unwrap();
            let msg: Result<OrderResponse, _> = serde_json::from_str(&msg[..]);
            if let Ok(m) = msg {
                if m.success {
                    return Ok(());
                } else {
                    warn!("cancel_order: {}", m.result);
                }
            }
        } else {
            error!("cancel_order error: {:?}", res);
        }
        return Err(());
    }
}

struct ControllerToggle {
    dirty: bool,
    enabled: bool,
}

impl ControllerToggle {
    pub fn new() -> Self {
        Self {
            dirty: false,
            enabled: false,
        }
    }

    pub fn enable(&mut self) {
        self.dirty = !self.enabled;
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.dirty = !self.enabled;
        self.enabled = false;
    }

    #[inline]
    pub fn state(&self) -> bool {
        self.enabled
    }

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[inline]
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    #[inline]
    pub fn set_dirty(&mut self) {
        self.dirty = true;
    }
}

struct WebSocketController {
    markets: Vec<String>,
    pub channel_orders: ControllerToggle,
    pub channel_orderbook: ControllerToggle,
    pub channel_ticker: ControllerToggle,
    pub channel_trades: ControllerToggle,
    pub should_terminate: bool,
}

impl WebSocketController {
    pub fn new() -> Self {
        Self {
            channel_orders: ControllerToggle::new(),
            channel_orderbook: ControllerToggle::new(),
            channel_ticker: ControllerToggle::new(),
            channel_trades: ControllerToggle::new(),
            markets: Vec::new(),
            should_terminate: false,
        }
    }

    pub fn add_market(&mut self, market: &str) {
        self.markets.push(String::from(market));
    }

    pub async fn resubscribe(&mut self, writer: &mut WsWriter) {
        self.channel_orders.set_dirty();
        self.channel_orderbook.set_dirty();
        self.channel_ticker.set_dirty();
        self.channel_trades.set_dirty();
        self.check_for_changes(writer).await;
    }

    pub async fn check_for_changes(&mut self, writer: &mut WsWriter) {
        if self.channel_orders.is_dirty() {
            if let Err(e) = self
                .subscribe_orders(writer, self.channel_orders.state())
                .await
            {
                error!("subscribe_orders: {:?}", e);
            }
            self.channel_orders.clear_dirty()
        }
        if self.channel_orderbook.is_dirty() {
            for market in self.markets.iter() {
                if let Err(e) = self
                    .subscribe_orderbook(writer, self.channel_orderbook.state(), market)
                    .await
                {
                    error!("subscribe_orders: {:?}", e);
                }
            }
            self.channel_orderbook.clear_dirty()
        }
        if self.channel_ticker.is_dirty() {
            for market in self.markets.iter() {
                if let Err(e) = self
                    .subscribe_ticker(writer, self.channel_ticker.state(), market)
                    .await
                {
                    error!("subscribe_ticker: {:?}", e);
                }
            }
            self.channel_ticker.clear_dirty()
        }
        if self.channel_trades.is_dirty() {
            for market in self.markets.iter() {
                if let Err(e) = self
                    .subscribe_trades(writer, self.channel_trades.state(), market)
                    .await
                {
                    error!("subscribe_trades: {:?}", e);
                }
            }
            self.channel_trades.clear_dirty()
        }
    }

    /// Authenticate the websocket to enable access to private channels.
    pub async fn authenticate(&self, writer: &mut WsWriter, api_key : &str, api_secret : &str) -> Result<(), ()> {
        let (ts, ts_s) = get_timestamp();
        let payload = FtxLogin {
            op: String::from("login"),
            args: FtxLoginSignature {
                key: String::from(api_key),
                sign: build_ws_signature(&ts_s, api_secret),
                time: ts,
            },
        };
        let payload = serde_json::to_string(&payload).unwrap();
        let msg = Message::from(payload);
        writer.send(msg).await.map_err(|e| {
            error!("authenticate: {:?}", e);
            ()
        })
    }

    pub async fn subscription_request(
        &self,
        writer: &mut WsWriter,
        channel: &str,
        enable: bool,
        market: &str,
    ) -> Result<(), ()> {
        let cmd = SubscriptionMgmt {
            channel,
            market,
            op: if enable {
                CMD_SUBSCRIBE
            } else {
                CMD_UNSUBSCRIBE
            },
        };
        let msg = Message::from(serde_json::to_string(&cmd).unwrap());
        let result = writer.send(msg).await;
        if result.is_err() {
            error!("subscription error for {}: {:?}", channel, result);
        }
        result.map_err(|_| ())
    }
    /// Manage whether our order updates are received on the private channel.
    pub async fn subscribe_orders(&self, writer: &mut WsWriter, enable: bool) -> Result<(), ()> {
        self.subscription_request(writer, "orders", enable, "")
            .await
    }

    /// Manage whether order book updates are received on the channel.
    pub async fn subscribe_orderbook(
        &self,
        writer: &mut WsWriter,
        enable: bool,
        market: &str,
    ) -> Result<(), ()> {
        self.subscription_request(writer, "orderbook", enable, market)
            .await
    }

    /// Manage whether the price ticker is received on the channel.
    /// The ticker_code is a valid FTX ticker and set enable to true to subscribe
    /// or false to unsubscribe.
    pub async fn subscribe_ticker(
        &self,
        writer: &mut WsWriter,
        enable: bool,
        market: &str,
    ) -> Result<(), ()> {
        self.subscription_request(writer, "ticker", enable, market)
            .await
    }

    /// Manage whether trades are received on the channel.
    /// The ticker_code is a valid FTX ticker and set enable to true to subscribe
    /// or false to unsubscribe.
    pub async fn subscribe_trades(
        &self,
        writer: &mut WsWriter,
        enable: bool,
        market: &str,
    ) -> Result<(), ()> {
        self.subscription_request(writer, "trades", enable, market)
            .await
    }
}
pub struct FtxManager {
    // into_channel_to_websocket: mpsc::Sender<tokio_tungstenite::tungstenite::Message>,
    market: String,
    order_channel: broadcast::Sender<UpdateMessage>,
    ws_controller: Arc<Mutex<WebSocketController>>,
    rest_api: RestApi,
}

impl FtxManager {
    /// Returns an interface to the exchange REST API
    pub fn api(&self) -> &RestApi {
        &self.rest_api
    }

    /// Enable receipt of the private orders channel.
    pub async fn orders_subscription(&self, enable: bool) {
        if enable {
            self.ws_controller.lock().await.channel_orders.enable();
        } else {
            self.ws_controller.lock().await.channel_orders.disable();
        }
    }

    pub async fn ticker_subscription(&self, enable: bool) {
        if enable {
            self.ws_controller.lock().await.channel_ticker.enable();
        } else {
            self.ws_controller.lock().await.channel_ticker.disable();
        }
    }

    pub async fn trades_subscription(&self, enable: bool) {
        if enable {
            self.ws_controller.lock().await.channel_trades.enable();
        } else {
            self.ws_controller.lock().await.channel_trades.disable();
        }
    }

    pub async fn terminate(&self) {
        self.ws_controller.lock().await.should_terminate = true;
    }

    pub async fn new(api_key: &str, api_secret: &str, ticker: &str) -> Self {
        // Create the broadcast channel for sending updates to interested workers.
        let (tx, mut _rx) = broadcast::channel::<UpdateMessage>(512);

        // Create handler object for the FTX connection to return to caller,
        let ftx_mgr = Self {
            market: String::from(ticker),
            order_channel: tx,
            ws_controller: Arc::new(Mutex::new(WebSocketController::new())),
            rest_api: RestApi::new(api_key, api_secret),
        };
        ftx_mgr.ws_controller.lock().await.add_market(ticker);

        // Create a worker to listen to the FTX websocket and process messages
        let broadcast_channel = ftx_mgr.order_channel.clone();
        let controller = ftx_mgr.ws_controller.clone();
        let key = String::from(api_key);
        let secret =   String::from(api_secret);
        tokio::spawn(async move {
            ftx_data_worker(broadcast_channel, controller, &FTX_WEBSOCKET_URL, &key, &secret).await
        });
        ftx_mgr
    }

    fn get_order_channel(&self) -> broadcast::Receiver<UpdateMessage> {
        self.order_channel.subscribe()
    }

    /// Not implemented.
    pub fn orderbook_crc(&self) -> i32 {
        0
    }
}

// #[async_trait]
// impl lob::LOBOrderManagement for FtxManager {
//     async fn cancel_order(&self, order_id: lob::OrderId) -> Result<(), ()> {
//         let order_id = order_id as FtxOrderID;
//         self.rest_api.cancel_order(order_id).await
//     }

//     async fn limit_order(
//         &self,
//         side: lob::SideOfBook,
//         price: lob::Price,
//         size: lob::Volume,
//         post_only: bool,
//         immediate_or_cancel: bool,
//     ) -> Result<lob::OrderId, FailureReason> {
//         debug_assert!(!(post_only && immediate_or_cancel));
//         let response = self
//             .rest_api
//             .limit_order(
//                 &self.market,
//                 if side == lob::SideOfBook::BID {
//                     "buy"
//                 } else {
//                     "sell"
//                 },
//                 lob::from_price(price),
//                 "limit",
//                 size,
//                 false,
//                 immediate_or_cancel,
//                 post_only,
//                 None,
//             )
//             .await;
//         response.map(|i| i.try_into().unwrap()).map_err(|e| {
//             if e == "Not enough balances" {
//                 FailureReason::InsufficientFunds
//             } else {
//                 FailureReason::NetworkError
//             }
//         })
//     }
// }

mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use tokio::time::{sleep, Duration};
    #[allow(unused_imports)]
    use tokio_test;

    #[test]
    fn test_build_signature_get() {
        let signature = build_signature(
            "T4lPid48QtjNxjLUFOcUZghD7CUJ7sTVsfuvQZF2",
            "1588591511721",
            "GET",
            "/api/markets",
            None,
        );
        assert_eq!(
            signature,
            "dbc62ec300b2624c580611858d94f2332ac636bb86eccfa1167a7777c496ee6f"
        );
    }

    #[test]
    fn test_build_signature_post() {
        let payload = r#"{"market": "BTC-PERP", "side": "buy", "price": 8500, "size": 1, "type": "limit", "reduceOnly": false, "ioc": false, "postOnly": false, "clientId": null}"#;
        let signature = build_signature(
            "T4lPid48QtjNxjLUFOcUZghD7CUJ7sTVsfuvQZF2",
            "1588591856950",
            "POST",
            "/api/orders",
            Some(payload),
        );
        assert_eq!(
            signature,
            "c4fbabaf178658a59d7bbf57678d44c369382f3da29138f04cd46d3d582ba4ba"
        );
    }

    #[test]
    fn test_build_signature_ws() {
        let timestamp = "1557246346499";
        let secret = "Y2QTHI23f23f23jfjas23f23To0RfUwX3H42fvN-";
        let signature = build_ws_signature(timestamp, secret);
        assert_eq!(
            signature,
            "d10b5a67a1a941ae9463a60b285ae845cdeac1b11edc7da9977bef0228b96de9"
        );
    }

    // #[test]
    // fn test_get_markets() {
    //     let f = RestApi::new();
    //     let s = tokio_test::block_on(f.get_markets());
    //     if let Ok(m) = s {
    //         assert!(m.len() > 0);
    //     }
    // }

    // #[test]
    // fn test_cancel_order() {
    //     let f = RestApi::new();

    //     let rt = tokio::runtime::Builder::new_multi_thread()
    //     .enable_all()
    //     .build()
    //     .unwrap();

    //     rt.block_on(async {

    //         let s = f.limit_order("BTC-PERP", "buy", 20300.0, "limit", 0.0001, false, false, false, None).await;
    //         let id = s.unwrap();
    //         sleep(Duration::from_secs(10)).await;
    //         let s = f.cancel_order(id).await;
    //         println!("{:?}", s);
    //     })
    // }
}
