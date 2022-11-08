use tokio::time::{interval_at, Duration, Instant};
use ftx_async::ws::{WebsocketManager, UpdateMessage};
use ftx_async::tests::get_test_credentials;

#[test]
fn test_trades_channel() {
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
                
        ftx.subscribe_channel_trades(true).await;
        tokio::select! {
            _ = test_timeout.tick() => {
                panic!("Timeout - no trade received from FTX. ");
            }
            Ok(msg) = listener.recv() => {
                if let UpdateMessage::Trade {market, id : _, price : _, size : _, time : _, liquidation : _, side : _}= msg {
                    assert_eq!(market, TEST_MARKET);
                }    
            }
        }
    });
}
