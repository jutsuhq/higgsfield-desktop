//! Jutsu Studio desktop entry point.

// Hides the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    jutsu_studio_lib::run()
}
