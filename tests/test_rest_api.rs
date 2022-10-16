use std::env;
use ftx_async::rest::RestApi;

#[test]
fn test_get_markets() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let api_key = env::var("FTX_API_KEY").unwrap();
        let secret = env::var("FTX_SECRET").unwrap();
        let api = RestApi::new(&api_key, &secret);
        let o = api.get_markets().await.unwrap();
        assert!(o.len() > 0);
    });
}
