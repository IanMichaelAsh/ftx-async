# ftx-async
Unofficial Rust implementation of an asynchronous Websocket and REST client for the FTX crypto exchange 

![ci](https://github.com/IanMichaelAsh/ftx-async/actions/workflows/ci.yml/badge.svg)

>06/11/2022 - *** WARNING *** This code base is now available as a crate. It remains alpha.

<h1>Example</h1>
A basic ticker listener using a FTX websocket.
Make sure you include tokio and ftx-async in your Cargo.toml:

```toml
[dependencies]
tokio = {version = "*"}
ftx_async = {version = "*"}
```
Then, on your main.rs:
```rust, no_run
use ftx_async::ws::{UpdateMessage, WebsocketManager};
use tokio::signal;

#[tokio::main]
async fn main() {
    let api_key = ""; // Set a valid FTX API key!
    let api_secret = ""; // Set a valid FTX secret key!

    let ftx = WebsocketManager::new(&api_key, &api_secret, "BTC-PERP").await;

    let mut listener = ftx.get_order_channel();
    ftx.subscribe_channel_ticker(true).await;

    let mut terminated = false;
    while !terminated {
        tokio::select! {
            Ok(msg) = listener.recv() => {
                if let UpdateMessage::BestPrice {market, bid, ask, bid_size : _, ask_size : _, last_trade : _}= msg {
                    print!("\r{market}: Bid: {:.0}     -     Ask: {:.0}", bid.unwrap(), ask.unwrap());
                }
            }

            _ = signal::ctrl_c() => {
                terminated = true;
            }
        }
    }
}
```

<h1>Running Integration Tests</h1>
The crate integration tests use an environment variables to look up credentials in order to establish a connection with the FTX exchange. A read-only key should be created on FTX and its details should be set into 'FTX_API_KEY' and 'FTX_SECRET' environment variables on the machine that will run the tests.

