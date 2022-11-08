#![warn(missing_docs)]
use const_format::concatcp;
use hmac::{Hmac, Mac};
use reqwest::Response;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, warn};

use crate::interface::{
    AccountInfo, AccountInfoResponse, FtxId, FtxPrice, FtxSize, Market, OrderResponse, PlaceOrder,
    PlaceOrderResponse, RestResponseMarketList, RestResponseOrderList, WalletBalances,
};

pub use crate::ws::{OrderType, SideOfBook};

const FTX_REST_URL: &str = "https://ftx.com";
const URI_GET_ACCOUNT_INFO: &str = "/api/account";
const URL_GET_ACCOUNT_INFO: &str = concatcp!(FTX_REST_URL, URI_GET_ACCOUNT_INFO);
const URI_GET_WALLET: &str = "/api/wallet/balances";
const URL_GET_WALLET: &str = concatcp!(FTX_REST_URL, URI_GET_WALLET);
const URI_ORDERS: &str = "/api/orders";
const URL_ORDERS: &str = concatcp!(FTX_REST_URL, URI_ORDERS);
const URI_MARKETS: &str = "/api/markets";
const URL_MARKETS: &str = concatcp!(FTX_REST_URL, URI_MARKETS);

/// Returns the current time as an Unix EPOX timestamp in milliseconds and as a string.
fn get_timestamp() -> (u128, String) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    (ts, ts.to_string())
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

/// An asynchronouse client to make REST API requests to FTX.
///
/// Example
/// Initialise the REST client and retrieve the list of available markets.
/// ```rust
/// #[tokio::main]
/// async fn main() {
///     let api_key = ""; // A valid api key for a FTX account
///     let api_secret = ""; // The secret corresponding to the provided API key
///     let client = ftx_async::rest::RestApi::new(api_key, api_secret);
///     let markets = client.get_markets().await.unwrap();
/// }
pub struct RestApi {
    client: reqwest::Client,
    api_key: String,
    api_secret: String,
}

impl RestApi {
    /// Construct a new RestApi.
    pub fn new(api_key: &str, api_secret: &str) -> RestApi {
        Self {
            client: reqwest::Client::builder()
                .tcp_nodelay(true)
                .build()
                .unwrap(),
            api_key: String::from(api_key),
            api_secret: String::from(api_secret),
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
    pub async fn get_markets(&self) -> Result<Vec<Market>, ()> {
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

    /// Submit an order to the ['Place Order'](https://docs.ftx.com/reference/place-order) endpoint.  
    ///
    /// * 'market' - Market to trade. e.g. BTC-PERP
    /// * 'side' - SideOfBook::BUY or SideOfBook::SELL
    /// * 'price' - Order price; Ignored for market orders
    /// * 'order_type' - OrderType::MARKET or OrderType::LIMIT
    /// * 'size' - Order size
    /// * 'reduce_only' - Only place order if it will reduce current position size
    /// * 'ioc' - Immediate-or-cancel
    /// * 'post_only' - Only place order if it will enter the orderbook (maker only)
    /// * 'client_id' - (Optional) Client-assigned order iD; Max length 64 characters; must be unique on a per subaccount basis
    pub async fn place_order(
        &self,
        market: &str,
        side: SideOfBook,
        price: FtxPrice,
        order_type: OrderType,
        size: FtxSize,
        reduce_only: bool,
        ioc: bool,
        post_only: bool,
        client_id: Option<&str>,
    ) -> Result<FtxId, String> {
        let body = PlaceOrder {
            market,
            side: match side {
                SideOfBook::BUY => "buy",
                SideOfBook::SELL => "sell",
            },
            price: match order_type {
                OrderType::MARKET => None,
                OrderType::LIMIT => Some(price),
            },
            order_type: match order_type {
                OrderType::MARKET => "market",
                OrderType::LIMIT => "limit",
            },
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
        let signature = build_signature(&self.api_secret, &ts, "POST", endpoint, Some(&payload));

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
                    let id = m.result.expect("Unable to convert FTX ID to u64").id;
                    return Ok(id);
                } else {
                    return Err(m.error.unwrap_or(String::from("No FTX error msg supplied")));
                }
            } else {
                return Err(String::from("Serde error unpacking place order response"));
            }
        }
        return Err(String::from("Unknown place_order error"));
    }

    /// Submit an order to the ['Cancel Order'](https://docs.ftx.com/reference/cancel-order) endpoint.  
    pub async fn cancel_order(&self, order_id: FtxId) -> Result<(), ()> {
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
}
