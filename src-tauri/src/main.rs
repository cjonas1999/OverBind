// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::env;
#[cfg(target_os = "windows")]
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::api::path::data_dir;
mod key_interceptor;

use crate::key_interceptor::KeyInterceptor;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize, Deserialize)]
struct KeyConfig {
    keycode: String,
    result_type: String,
    result_value: i32,
}

#[derive(Clone)]
struct KeyInterceptorState(Arc<Mutex<KeyInterceptor>>);

#[tauri::command]
fn start_interception(state: State<KeyInterceptorState>) -> Result<(), String> {
    let mut interceptor = state.0.lock().unwrap();
    interceptor.start().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn stop_interception(state: State<KeyInterceptorState>) {
    let interceptor = state.0.lock().unwrap();
    interceptor.stop();
}

#[tauri::command]
fn is_interceptor_running(state: State<KeyInterceptorState>) -> bool {
    let interceptor = state.0.lock().unwrap();
    interceptor.is_running()
}

fn get_config_path() -> Result<PathBuf, String> {
    match data_dir() {
        Some(mut path) => {
            path.push("OverBind"); // Use your app's unique folder name
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?; // Create the dir if it doesn't exist
            path.push("OverBind_conf.json");
            Ok(path)
        }
        None => Err("Failed to get user data directory".into()),
    }
}

#[tauri::command]
fn read_config() -> Result<Vec<KeyConfig>, String> {
    let config_path = get_config_path()?;
    let file = File::open(config_path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);

    let configs: Vec<KeyConfig> = serde_json::from_reader(reader).map_err(|e| e.to_string())?;

    Ok(configs)
}

#[tauri::command]
fn save_config(configs: Vec<KeyConfig>) -> Result<(), String> {
    let config_path = get_config_path()?;

    let mut file = File::create(config_path).map_err(|e| e.to_string())?;

    let json = serde_json::to_string_pretty(&configs).map_err(|e| e.to_string())?;

    file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;

    Ok(())
}

fn ensure_config_file_exists() -> Result<(), String> {
    let config_path = get_config_path()?;
    if !config_path.exists() {
        // Assuming 'include_str!' is used to include the file contents in the binary
        let default_contents = include_str!("../OverBind_conf.json");
        fs::write(config_path, default_contents).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    let _ = ensure_config_file_exists();
    let interceptor = KeyInterceptor::new();
    let interceptor_arc = Arc::new(Mutex::new(interceptor));
    let interceptor_state = KeyInterceptorState(interceptor_arc.clone());
    {
        let mut interceptor = interceptor_arc.lock().unwrap();
        interceptor.initialize().unwrap();
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(interceptor_state)
        .invoke_handler(tauri::generate_handler![
            read_config,
            save_config,
            start_interception,
            stop_interception,
            is_interceptor_running,
        ])
        .build(tauri::generate_context!())
        .unwrap() // Handle the error using unwrap
        .run(|_app_handle, _event| {
            // Here you can handle specific events if needed
        });
}
