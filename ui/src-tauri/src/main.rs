// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::fs::{self, File};
use std::io::{BufRead, Read, Write};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::{env, io};
use tauri::api::path::data_dir;
// For working with the environment, including the current directory
use tauri::command;

struct AppState {
    process_handle: Option<Child>,
}

impl AppState {
    fn new() -> Self {
        AppState {
            process_handle: None,
        }
    }
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn get_config_path() -> Result<PathBuf, String> {
    match data_dir() {
        Some(mut path) => {
            path.push("OverBind"); // Use your app's unique folder name
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?; // Create the dir if it doesn't exist
            path.push("OverBind_conf.txt");
            Ok(path)
        }
        None => Err("Failed to get user data directory".into()),
    }
}

#[command(async)]
fn start_process(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Result<String, String> {
    println!("Attempting to start process");

    let mut app_state = state.lock().unwrap();
    if app_state.process_handle.is_some() {
        println!("Process is already running");
        return Err("Process is already running".into());
    }

    // Adjust the path to the executable relative to the current working directory
    let relative_path = "OverBind_process.exe"; // Update this path accordingly
    let exe_path = match env::current_dir() {
        Ok(mut path) => {
            path.push(relative_path);
            path
        }
        Err(_) => return Err("Failed to get current directory".into()),
    };

    let mut child = Command::new(exe_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| {
            println!("Failed to start C++ executable: {}", e);
            format!("Failed to start C++ executable: {}", e)
        })?;

    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
    app_state.process_handle = Some(child);

    // Spawn a thread to asynchronously check for immediate errors
    std::thread::spawn(move || {
        let mut err = String::new();
        let mut stderr_reader = io::BufReader::new(stderr);
        stderr_reader.read_to_string(&mut err).unwrap();
        if !err.is_empty() {
            println!("Error from C++ executable: {}", err);
        }
    });

    println!("Process started successfully");
    Ok("Process started successfully".into())
}

#[command]
fn is_process_running(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> bool {
    let app_state = state.lock().unwrap();
    app_state.process_handle.is_some()
}

#[command]
fn stop_process(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Result<String, String> {
    println!("Attempting to stop process");
    let mut app_state = state.lock().unwrap();

    if let Some(mut child) = app_state.process_handle.take() {
        match child.kill() {
            Ok(_) => {
                let _ = child.wait(); // Wait for the process to terminate
                println!("Process stopped successfully");
                Ok("Process stopped successfully".into())
            }
            Err(e) => {
                println!("Failed to stop process: {}", e);
                Err(format!("Failed to stop process: {}", e))
            }
        }
    } else {
        println!("No process is running");
        Err("No process is running".into())
    }
}

#[command]
fn read_config() -> Result<Vec<u32>, String> {
    let config_path = get_config_path()?;
    let file = File::open(config_path).map_err(|e| e.to_string())?;
    let reader = io::BufReader::new(file);

    let mut numbers = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let num = u32::from_str_radix(&line, 16).map_err(|e| e.to_string())?;
        numbers.push(num);
    }

    Ok(numbers)
}

#[command]
fn save_config(codes: Vec<u32>) -> Result<(), String> {
    let config_path = get_config_path()?;

    let mut file = File::create(config_path).map_err(|e| e.to_string())?;

    for num in codes {
        writeln!(file, "{:X}", num).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn ensure_config_file_exists() -> Result<(), String> {
    let config_path = get_config_path()?;
    if !config_path.exists() {
        // Assuming 'include_str!' is used to include the file contents in the binary
        let default_contents = include_str!("../OverBind_conf.txt");
        fs::write(config_path, default_contents).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    let app_state = Arc::new(Mutex::new(AppState::new()));
    let app_state_clone = app_state.clone(); // Clone app_state for use in the closure
    let _ = ensure_config_file_exists();

    tauri::Builder::default()
        .manage(app_state) // Pass the cloned state to the Tauri app
        .invoke_handler(tauri::generate_handler![
            start_process,
            is_process_running,
            stop_process,
            read_config,
            save_config,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(move |_app_handle, e| match e {
            tauri::RunEvent::WindowEvent { event, .. } => match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    println!("Detected tauri::RunEvent::WindowEvent");
                    let mut app_state = app_state_clone.lock().unwrap();
                    if let Some(mut child) = app_state.process_handle.take() {
                        match child.kill() {
                            Ok(_) => {
                                let _ = child.wait(); // Ensure the process is terminated
                                println!("Child process killed successfully");
                            }
                            Err(err) => println!("Failed to kill child process: {}", err),
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        });
}
