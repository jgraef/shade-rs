pub mod app;
pub mod error;
pub mod graphics;
pub mod utils;

use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, Registry};
use tracing_wasm::{WASMLayer, WASMLayerConfigBuilder};
use wasm_bindgen::{prelude::wasm_bindgen, JsCast};

use crate::app::App;

fn main() {
    let _ = tracing::subscriber::set_global_default(
        Registry::default().with(WASMLayer::new(WASMLayerConfigBuilder::new()
        .set_max_level(Level::DEBUG)
        .build())));
    console_error_panic_hook::set_once();

    tracing::info!("shade-rs initialized");
}

#[wasm_bindgen]
pub fn mount_to(id: &str) {
    tracing::info!("mounting shade-rs");

    let root = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document")
        .get_element_by_id(id)
        .expect("root element not found")
        .dyn_into()
        .unwrap();

    leptos::mount_to(root, App);
}
