use ftx_async::tests::get_test_credentials;
#[allow(unused_imports)]
use ftx_async::rest::{OrderType, RestApi, SideOfBook};

#[test]
fn test_get_markets() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let (api_key, api_secret) = get_test_credentials();
    let api = RestApi::new(&api_key, &api_secret);
    rt.block_on(async {
        let o = api.get_markets().await.unwrap();
        assert!(o.len() > 0);
    });
}

// #[test]
// fn test_place_order() {
//     let rt = tokio::runtime::Builder::new_multi_thread()
//         .enable_all()
//         .build()
//         .unwrap();

//     // Key needs write privileges on FTX to work.
//     let (api_key, api_secret) = get_test_credentials();
//     let api = RestApi::new(&api_key, &api_secret);

//     let result = rt.block_on(async {
//         api.place_order(
//             "BTC-PERP",
//             SideOfBook::BUY,
//             15000.0,
//             OrderType::LIMIT,
//             0.001,
//             false,
//             false,
//             false,
//             None,
//         )
//         .await
//     });
//     match result {
//         Ok(r) => println!("{:?}", r),
//         Err(e) => println!("{:?}", e),
//     };
// }
