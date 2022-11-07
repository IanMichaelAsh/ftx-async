#![warn(rustdoc::missing_crate_level_docs)]
//! Provides access to the FTX REST API and web socket interfaces.
//!

/// Serde-enabled structure definitions for FTX API calls and response payloads..
pub mod interface;
/// Asynchronous interface to call the ['FTX REST API'](https://docs.ftx.com/reference/rest-api).
/// 
/// The module makes use of ['reqwest'](https://docs.rs/reqwest/latest/reqwest/) as an HTTP client and ['serde'](https://serde.rs/) for marshalling data in and out of the socket.
pub mod rest;
/// The ws module enables connections to the ['FTX websocket API'](https://docs.ftx.com/reference/websocket-overview). 
///
/// The websocket manager provides complete management of the websocket, including authentication, reconnection and keep-alives.
/// 
/// The module depends on ['tokio-tungstenite'](https://docs.rs/tokio-tungstenite/latest/tokio_tungstenite/) for low-level management of the websocket and ['serde'](https://serde.rs/) for marshalling data in and out of the socket.
pub mod ws;

/// Integration tests and test helper functions.
pub mod tests {
    use std::env;

    pub fn get_test_credentials() -> (String, String) {
        let api_key =
            env::var("FTX_API_KEY").expect("Could not find FTX_API_KEY environment variable.");
        let secret =
            env::var("FTX_SECRET").expect("Could not find FTX_SECRET environment variable.");
        (api_key, secret)
    }
}
