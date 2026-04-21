#![allow(dead_code)]

mod app;
mod data;

use app::App;
use leptos::prelude::*;

fn main() {
    console_log::init_with_level(log::Level::Debug).expect("Failed to init logging");
    leptos::mount::mount_to_body(|| {
        view! { <App/> }
    })
}
