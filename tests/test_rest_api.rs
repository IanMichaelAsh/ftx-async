use ftx_async::tests::get_test_credentials;
use ftx_async::rest::RestApi;

#[test]
fn test_get_markets() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let (api_key, api_secret) = get_test_credentials();
        let api = RestApi::new(&api_key, &api_secret);
        let o = api.get_markets().await.unwrap();
        assert!(o.len() > 0);
    });
}
