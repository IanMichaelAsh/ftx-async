use tokio::time::{interval_at, Duration, Instant};
use ftx_async::ws::{WebsocketManager, UpdateMessage};
use ftx_async::tests::get_test_credentials;

#[test]
fn test_ticker_channel() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let (api_key, api_secret) = get_test_credentials();
        let ftx = WebsocketManager::new(
            &api_key,
            &api_secret,
            "BTC-PERP",
        )
        .await;

        let mut listener = ftx.get_order_channel();
        let mut test_timeout = interval_at(Instant::now() + Duration::from_secs(15), Duration::from_secs(15));
                
        ftx.ticker_subscription(true).await;
        tokio::select! {
            _ = test_timeout.tick() => {
                panic!("Timeout - no response from FTX. ");
            }
            Ok(msg) = listener.recv() => {
                if let UpdateMessage::BestPrice {bid, ask, bid_size : _, ask_size : _, last_trade : _}= msg {
                    assert!(bid.is_some());
                    assert!(ask.is_some());
                }    
            }
        }
    });
}
