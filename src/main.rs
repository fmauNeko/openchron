//! Openchron: a cross-platform configuration tool for Keychron keyboards and mice, built with GPUI.

// Release builds on Windows must not spawn a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    gpui_kit::application()
        .with_assets(openchron::assets::AppAssets)
        .run(openchron::app::run);
}
