use ftx_async::ws::FtxManager;

#[test]
fn test_ticker_channel() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        // let subscriber = tracing_subscriber::FmtSubscriber::new();
        // tracing::subscriber::set_global_default(subscriber).unwrap();
        // info!("full_integration_test started");
        let ftx = FtxManager::new(
            "-LN75anxaQPvbD_p_P8p6wDqbAx39j6ON2zUoKZk",
            "XUgWdstqn0cMsxkxheGlhOdajIHea-7tChABM8Xr",
            "BTC-PERP",
        )
        .await;
    });
}
