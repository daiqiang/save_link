use savelink_core::baidu_oauth::{BaiduOAuthConfig, BaiduOAuthResult};
use std::time::Duration;

include!(concat!(env!("OUT_DIR"), "/baidu_oauth_config.rs"));

pub fn baidu_oauth_config() -> BaiduOAuthResult<BaiduOAuthConfig> {
    BaiduOAuthConfig::with_options(
        BAIDU_APP_KEY,
        BAIDU_SECRET_KEY,
        BAIDU_REDIRECT_URI,
        BAIDU_SCOPE,
        "https://openapi.baidu.com",
        Duration::from_secs(120),
    )
}
