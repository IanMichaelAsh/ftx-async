use tokio::time::{interval_at, Duration, Instant};
use ftx_async::ws::{WebsocketManager, UpdateMessage};
use ftx_async::tests::get_test_credentials;

#[test]
fn test_ticker_channel() {
    const TEST_MARKET : &str = "BTC-PERP";
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let (api_key, api_secret) = get_test_credentials();
        let ftx = WebsocketManager::new(
            &api_key,
            &api_secret,
            TEST_MARKET,
        )
        .await;

        let mut listener = ftx.get_order_channel();
        let mut test_timeout = interval_at(Instant::now() + Duration::from_secs(15), Duration::from_secs(15));
                
        ftx.subscribe_channel_ticker(true).await;
        tokio::select! {
            _ = test_timeout.tick() => {
                panic!("Timeout - no response from FTX. ");
            }
            Ok(msg) = listener.recv() => {
                if let UpdateMessage::Ticker {market, bid, ask, bid_size : _, ask_size : _, last_trade : _}= msg {
                    assert_eq!(market, TEST_MARKET);
                    assert!(bid.is_some());
                    assert!(ask.is_some());
                }    
            }
        }
    });
}
