use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use hmac::{Hmac, Mac};
use num_traits::Zero;
use serde::Serialize;
use sha2::Sha256;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Mutex};
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

use crate::interface::{
    FtxLogin, FtxLoginSignature, FtxMessage, PartialData, UpdateData, OrderBookUpdate, FtxSize, FtxOrderId, FtxPrice 
};
use crate::rest::RestApi;



type WsWriter = SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Message,
>;

type WsReader = SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

const FTX_WEBSOCKET_URL: &str = "wss://ftx.com/ws/";
const PING_MSG: &str = r#"{"op":"ping"}"#;
const CMD_SUBSCRIBE: &str = "subscribe";
const CMD_UNSUBSCRIBE: &str = "unsubscribe";

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
        price: FtxPrice,
        size: FtxSize,
        timestamp: i64,
        side: SideOfBook,
    },
}

#[derive(Debug, Clone)]
pub enum FailureReason {
    NetworkError,
    InsufficientFunds,
    OrderCancelled,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SubscriptionMgmt<'a> {
    channel: &'a str,
    market: &'a str,
    op: &'a str,
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

async fn start_websocket(ws_url: &str) -> (WsWriter, WsReader) {
    let url = url::Url::parse(ws_url).unwrap();
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (ws_writer, ws_reader) = ws_stream.split();
    (ws_writer, ws_reader)
}

async fn authenticate(
    ws_controller: &Arc<Mutex<WebSocketController>>,
    writer: &mut WsWriter,
    api_key: &str,
    api_secret: &str,
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
    api_key: &str,
    api_secret: &str,
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
            if authenticate(&ws_controller, &mut ws_writer, api_key, api_secret)
                .await
                .is_ok()
            {
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
    pub async fn authenticate(
        &self,
        writer: &mut WsWriter,
        api_key: &str,
        api_secret: &str,
    ) -> Result<(), ()> {
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
pub struct WebsocketManager {
    order_channel: broadcast::Sender<UpdateMessage>,
    ws_controller: Arc<Mutex<WebSocketController>>,
    rest_api: RestApi,
}

impl WebsocketManager {
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
            order_channel: tx,
            ws_controller: Arc::new(Mutex::new(WebSocketController::new())),
            rest_api: RestApi::new(api_key, api_secret),
        };
        ftx_mgr.ws_controller.lock().await.add_market(ticker);

        // Create a worker to listen to the FTX websocket and process messages
        let broadcast_channel = ftx_mgr.order_channel.clone();
        let controller = ftx_mgr.ws_controller.clone();
        let key = String::from(api_key);
        let secret = String::from(api_secret);
        tokio::spawn(async move {
            ftx_data_worker(
                broadcast_channel,
                controller,
                &FTX_WEBSOCKET_URL,
                &key,
                &secret,
            )
            .await
        });
        ftx_mgr
    }

    /// Returns a receiver channel onto which all received messages from FTX will be transmitted.
    /// Note that data will only appear on the channel if there is an active message subscription (see orders_subscription(), trades_subscription(), etc. )
    #[allow(dead_code)]
    pub fn get_order_channel(&self) -> broadcast::Receiver<UpdateMessage> {
        self.order_channel.subscribe()
    }

    /// Not implemented.
    pub fn orderbook_crc(&self) -> i32 {
        0
    }
}

/// Returns the current time as an Unix EPOX timestamp in milliseconds and as a string.
fn get_timestamp() -> (u128, String) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    (ts, ts.to_string())
}

mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use tokio::time::{sleep, Duration};
    #[allow(unused_imports)]
    use tokio_test;

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
