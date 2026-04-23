// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // CRITICAL: Set these BEFORE Tauri/WebKit initializes
    // Fixes white screen / DRI2 EGL failures on Linux with NVIDIA GPU
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
    std::env::set_var("GALLIUM_DRIVER", "llvmpipe");

    app_lib::run();
}
