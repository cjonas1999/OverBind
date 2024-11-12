// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

extern crate dirs;
extern crate log;
extern crate simplelog;

#[cfg(target_os = "linux")]
use linux_key_interceptor::LinuxKeyInterceptor;

use once_cell::sync::Lazy;
use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
#[cfg(target_os = "windows")]
use windows_key_interceptor::WindowsKeyInterceptor;

use serde_json::Value;
use std::fs::{self, create_dir_all, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{env, panic};
mod key_interceptor;
mod linux_key_interceptor;
mod windows_key_interceptor;

use crate::key_interceptor::KeyInterceptorTrait;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewWindow};

static WINDOW: Lazy<Arc<Mutex<Option<WebviewWindow>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

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
    force_cursor: bool,
}

#[derive(Clone)]
struct AppSettingsState(Arc<Mutex<Settings>>);

fn start_key_interception(app: &tauri::AppHandle, state: &State<KeyInterceptorState>) {
    // app.tray_by_id("main_tray")
    //     .unwrap()
    //     .("disable")
    //     .unwrap()
    //     .as_menuitem()
    //     .unwrap()
    //     .set_enabled(true)
    //     .unwrap();

    // app.menu()
    //     .unwrap()
    //     .get("enable")
    //     .unwrap()
    //     .as_menuitem()
    //     .unwrap()
    //     .set_enabled(false)
    //     .unwrap();

    let mut interceptor = state.0.lock().unwrap();
    let _ = interceptor.start(app).map_err(|e| e.to_string());
}

fn stop_key_interception(app: &tauri::AppHandle, state: &State<KeyInterceptorState>) {
    // app.menu()
    //     .unwrap()
    //     .get("disable")
    //     .unwrap()
    //     .as_menuitem()
    //     .unwrap()
    //     .set_enabled(false)
    //     .unwrap();

    // app.menu()
    //     .unwrap()
    //     .get("enable")
    //     .unwrap()
    //     .as_menuitem()
    //     .unwrap()
    //     .set_enabled(true)
    //     .unwrap();

    let interceptor = state.0.lock().unwrap();
    interceptor.stop(app);
}

fn is_key_interception_running(state: &State<KeyInterceptorState>) -> bool {
    let interceptor = state.0.lock().unwrap();
    interceptor.is_running()
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
        let window = app.get_webview_window("main").unwrap();
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
    match dirs::data_dir() {
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
    match dirs::data_dir() {
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
    let log_file_path = dirs::data_dir().unwrap().join("OverBind").join("error.log");
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let main_window = app.get_webview_window("main").unwrap();
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

            let menu = MenuBuilder::new(app)
                .item(&MenuItemBuilder::with_id("disable", "Disable OverBind").build(app)?)
                .item(
                    &MenuItemBuilder::with_id("enable", "Enable OverBind")
                        .enabled(false)
                        .build(app)?,
                )
                .item(&MenuItemBuilder::with_id("settings", "Open OverBind Settings").build(app)?)
                .item(&MenuItemBuilder::with_id("exit", "Exit").build(app)?)
                .build()?;
            let tray = tauri::tray::TrayIconBuilder::with_id("main_tray")
                .menu(&menu)
                .on_tray_icon_event({
                    let app_handle = app.handle().clone();
                    move |_, event| match event {
                        tauri::tray::TrayIconEvent::DoubleClick { .. } => {
                            let window = app_handle.get_webview_window("main").unwrap();
                            if window.is_visible().unwrap() {
                                window.hide().unwrap();
                            } else {
                                window.show().unwrap();
                                window.set_focus().unwrap();
                            }
                        }
                        tauri::tray::TrayIconEvent::Click { .. } => {
                            // Check if key interception is running
                            let state = app_handle.state::<KeyInterceptorState>();
                            let guard = state.0.lock().unwrap();
                            let interceptor = guard.as_ref();
                            if interceptor.is_running() {
                                let _ = menu
                                    .get("disable")
                                    .unwrap()
                                    .as_menuitem()
                                    .unwrap()
                                    .set_enabled(false);
                                let _ = menu
                                    .get("enable")
                                    .unwrap()
                                    .as_menuitem()
                                    .unwrap()
                                    .set_enabled(true);
                            } else {
                                let _ = menu
                                    .get("disable")
                                    .unwrap()
                                    .as_menuitem()
                                    .unwrap()
                                    .set_enabled(true);
                                let _ = menu
                                    .get("enable")
                                    .unwrap()
                                    .as_menuitem()
                                    .unwrap()
                                    .set_enabled(false);
                            }
                        }
                        _ => {}
                    }
                })
                .on_menu_event(|app, event| match event.id.0.as_ref() {
                    "disable" => {
                        let state = app.state::<KeyInterceptorState>();
                        stop_key_interception(app, &state);

                        let window = app.get_webview_window("main").unwrap();
                        window.emit("tray_intercept_disable", "").unwrap();
                    }
                    "enable" => {
                        let state = app.state::<KeyInterceptorState>();
                        start_key_interception(app, &state);

                        let window = app.get_webview_window("main").unwrap();
                        window.emit("tray_intercept_enable", "").unwrap();
                    }
                    "settings" => {
                        let window = app.get_webview_window("main").unwrap();
                        window.show().unwrap();
                        window.set_focus().unwrap();
                    }
                    "exit" => {
                        std::process::exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            let resource_dir = app
                .path()
                .resource_dir()
                .expect("Failed to get resource dir");
            std::env::set_var("OVERBIND_RESOURCE_DIR", resource_dir);

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
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" {
                    let app = window.app_handle();
                    let app_state = app.state::<AppSettingsState>();
                    let settings = app_state.0.lock().unwrap();

                    if settings.close_to_tray == true {
                        window.hide().unwrap();
                        api.prevent_close();
                    }
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building OvrBind")
        .run(|_app_handle, _event| {
            // Here you can handle specific events if needed
        });
}
