// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    t210_connect_lib::run();  // Memanggil fungsi run() dari lib.rs
}