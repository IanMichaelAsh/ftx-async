pub mod ws;
pub mod rest;
pub mod interface;

pub mod tests {
    use std::env;

    pub fn get_test_credentials() -> (String, String) {
        let api_key = env::var("FTX_API_KEY").expect("Could not find FTX_API_KEY environment variable.");
        let secret = env::var("FTX_SECRET").expect("Could not find FTX_SECRET environment variable.");
        (api_key, secret)
    }
}