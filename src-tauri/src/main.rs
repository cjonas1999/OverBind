// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use linux_key_interceptor::LinuxKeyInterceptor;

extern crate log;
extern crate simplelog;

use once_cell::sync::Lazy;
use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};
#[cfg(target_os = "windows")]
use windows_key_interceptor::WindowsKeyInterceptor;

use serde_json::Value;
use std::fs::{self, create_dir_all, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{env, panic};
use tauri::api::path::data_dir;
mod key_interceptor;
mod linux_key_interceptor;
mod windows_key_interceptor;

use crate::key_interceptor::KeyInterceptorTrait;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State, Window};

static WINDOW: Lazy<Arc<Mutex<Option<Window>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

#[derive(Serialize, Deserialize)]
struct KeyConfig {
    keycode: String,
    result_type: String,
    result_value: i32,
}

#[derive(Clone)]
struct KeyInterceptorState(Arc<Mutex<Box<dyn KeyInterceptorTrait + Send>>>);

impl KeyInterceptorState {
    fn new(settings: Settings) -> Self {
        #[cfg(target_os = "windows")]
        let interceptor: Box<dyn KeyInterceptorTrait + Send> =
            Box::new(WindowsKeyInterceptor::new());

        #[cfg(target_os = "linux")]
        let interceptor: Box<dyn KeyInterceptorTrait + Send> = Box::new(LinuxKeyInterceptor::new());

        let interceptor_arc = Arc::new(Mutex::new(interceptor));
        {
            let mut interceptor = interceptor_arc.lock().unwrap();
            interceptor.initialize(&settings).unwrap();
        }
        Self(interceptor_arc.clone())
    }
}

#[derive(Deserialize, Clone)]
struct Settings {
    close_to_tray: bool,
    allowed_programs: Vec<String>,
    selected_input: Option<String>,
}

#[derive(Clone)]
struct AppSettingsState(Arc<Mutex<Settings>>);

fn start_key_interception(app: &tauri::AppHandle, state: &State<KeyInterceptorState>) {
    app.tray_handle()
        .set_menu(make_disable_tray_menu())
        .unwrap();

    let mut interceptor = state.0.lock().unwrap();
    let _ = interceptor.start().map_err(|e| e.to_string());
}

fn stop_key_interception(app: &tauri::AppHandle, state: &State<KeyInterceptorState>) {
    app.tray_handle().set_menu(make_enable_tray_menu()).unwrap();

    let interceptor = state.0.lock().unwrap();
    interceptor.stop();
}

fn is_key_interception_running(state: &State<KeyInterceptorState>) -> bool {
    let interceptor = state.0.lock().unwrap();
    interceptor.is_running()
}

fn make_disable_tray_menu() -> tauri::SystemTrayMenu {
    let disble_interception = tauri::CustomMenuItem::new("disable".to_string(), "Disable OverBind");
    let open_overbind_settings =
        tauri::CustomMenuItem::new("settings".to_string(), "Open OverBind Settings");
    let exit = tauri::CustomMenuItem::new("exit".to_string(), "Exit");

    tauri::SystemTrayMenu::new()
        .add_item(disble_interception)
        .add_item(open_overbind_settings)
        .add_item(exit)
}

fn make_enable_tray_menu() -> tauri::SystemTrayMenu {
    let enable_interception = tauri::CustomMenuItem::new("enable".to_string(), "Enable OverBind");
    let open_overbind_settings =
        tauri::CustomMenuItem::new("settings".to_string(), "Open OverBind Settings");
    let exit = tauri::CustomMenuItem::new("exit".to_string(), "Exit");

    tauri::SystemTrayMenu::new()
        .add_item(enable_interception)
        .add_item(open_overbind_settings)
        .add_item(exit)
}

#[tauri::command]
fn start_interception(
    app: tauri::AppHandle,
    key_interceptor_state: State<KeyInterceptorState>,
    settings_state: State<AppSettingsState>,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let settings = settings_state.0.lock().unwrap();
        let window = app.get_window("main").unwrap();
        if settings.selected_input == None {
            window.emit("settings_incomplete", true).unwrap();
        } else {
            window.emit("settings_incomplete", false).unwrap();
        }
    }
    start_key_interception(&app, &key_interceptor_state);

    Ok(())
}

#[tauri::command]
fn stop_interception(app: tauri::AppHandle, state: State<KeyInterceptorState>) {
    stop_key_interception(&app, &state)
}

#[tauri::command]
fn is_interceptor_running(state: State<KeyInterceptorState>) -> bool {
    is_key_interception_running(&state)
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

fn ensure_settings_file_exists() -> Result<(), String> {
    let config_path = get_app_settings_path()?;
    if !config_path.exists() {
        // Assuming 'include_str!' is used to include the file contents in the binary
        let default_contents = include_str!("../OverBind_app_settings.json");
        fs::write(config_path, default_contents).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn get_app_settings_path() -> Result<PathBuf, String> {
    match data_dir() {
        Some(mut path) => {
            path.push("OverBind"); // Use your app's unique folder name
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?; // Create the dir if it doesn't exist
            path.push("OverBind_app_settings.json");
            Ok(path)
        }
        None => Err("Failed to get user data directory".into()),
    }
}

fn read_settings() -> Result<Value, String> {
    let path = get_app_settings_path()?;
    println!("Reading settings from {}", path.display());
    let file = File::open(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);

    let configs: Value = serde_json::from_reader(reader).map_err(|e| e.to_string())?;

    Ok(configs)
}

#[tauri::command]
fn read_app_settings() -> Result<Value, String> {
    read_settings()
}

fn update_settings(settings: Value, state: &State<AppSettingsState>) {
    let mut settings_state = state.0.lock().unwrap();

    let new_settings: Settings = serde_json::from_value(settings).unwrap();

    *settings_state = new_settings;
}

#[tauri::command]
fn save_app_settings(settings: Value, state: State<AppSettingsState>) -> Result<(), String> {
    let path = get_app_settings_path()?;

    let mut file = File::create(path).map_err(|e| e.to_string())?;

    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;

    file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;

    update_settings(settings, &state);

    Ok(())
}

#[tauri::command]
fn list_inputs() -> Result<Vec<String>, String> {
    let mut inputs = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let input_dir = Path::new("/dev/input/by-id");
        if let Ok(entries) = fs::read_dir(input_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    inputs.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        };
        println!("Inputs: {:?}", inputs);
    }
    Ok(inputs)
}

fn main() {
    let log_file_path = data_dir().unwrap().join("OverBind").join("error.log");
    create_dir_all(log_file_path.parent().unwrap()).expect("Could not create log file");
    let log_file = File::create(log_file_path).expect("Could not create log file");
    CombinedLogger::init(vec![WriteLogger::new(
        LevelFilter::Info,
        Config::default(),
        log_file,
    )])
    .expect("Could not initialize logger");

    let _ = ensure_config_file_exists();
    let _ = ensure_settings_file_exists();

    let settings_json = read_settings().unwrap();
    let settings: Settings = serde_json::from_value(settings_json).unwrap();

    let interceptor_state = KeyInterceptorState::new(settings.clone());
    let settings_arc = Arc::new(Mutex::new(settings));
    let settings_state = AppSettingsState(settings_arc.clone());

    tauri::Builder::default()
        .setup(|app| {
            let main_window = app.get_window("main").unwrap();
            {
                let mut window_lock = WINDOW.lock().unwrap();
                *window_lock = Some(main_window.clone());
            }
            panic::set_hook(Box::new(|panic_info| {
                if let Ok(window_lock) = WINDOW.lock() {
                    if let Some(window) = &*window_lock {
                        window.emit("panic", panic_info.to_string()).unwrap();
                    }
                }
                log::error!("{:?}", panic_info);
                eprintln!("{:?}", panic_info);
            }));

            Ok(())
        })
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(interceptor_state)
        .manage(settings_state)
        .invoke_handler(tauri::generate_handler![
            read_config,
            save_config,
            read_app_settings,
            save_app_settings,
            start_interception,
            stop_interception,
            is_interceptor_running,
            list_inputs,
        ])
        .system_tray(tauri::SystemTray::new().with_menu(make_disable_tray_menu()))
        .on_window_event(|event| match event.event() {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let app = event.window().app_handle();
                let app_state = app.state::<AppSettingsState>();
                let settings = app_state.0.lock().unwrap();

                if settings.close_to_tray == true {
                    event.window().hide().unwrap();
                    api.prevent_close();
                }
            }
            _ => {}
        })
        .on_system_tray_event(|app, event| match event {
            tauri::SystemTrayEvent::DoubleClick {
                position: _,
                size: _,
                ..
            } => {
                let window = app.get_window("main").unwrap();
                if window.is_visible().unwrap() {
                    window.hide().unwrap();
                } else {
                    window.show().unwrap();
                    window.set_focus().unwrap();
                }
            }
            tauri::SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "disable" => {
                    let state = app.state::<KeyInterceptorState>();
                    stop_key_interception(app, &state);

                    let window = app.get_window("main").unwrap();
                    window.emit("tray_intercept_disable", "").unwrap();
                }
                "enable" => {
                    let state = app.state::<KeyInterceptorState>();
                    start_key_interception(app, &state);

                    let window = app.get_window("main").unwrap();
                    window.emit("tray_intercept_enable", "").unwrap();
                }
                "settings" => {
                    let window = app.get_window("main").unwrap();
                    window.show().unwrap();
                    window.set_focus().unwrap();
                }
                "exit" => {
                    std::process::exit(0);
                }
                _ => {}
            },
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building OvrBind")
        .run(|_app_handle, _event| {
            // Here you can handle specific events if needed
        });
}
