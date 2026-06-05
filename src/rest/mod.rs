pub mod app_state;
pub mod health;
pub mod post_proxy;
pub mod server;
pub use app_state::*;
pub use health::*;
pub use post_proxy::*;
pub use server::*;

pub fn request_is_streaming(body_bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body_bytes)
        .ok()
        .and_then(|v| v.get("stream").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}
