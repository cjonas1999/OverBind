// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use tauri::command;
use std::env; // For working with the environment, including the current directory

#[command(async)]
fn run_overbind() -> Result<String, String> {
    use std::process::{Command, Stdio};

    // Print the current working directory
    match env::current_dir() {
        Ok(pwd) => println!("Current working directory: {:?}", pwd),
        Err(e) => println!("Error getting current directory: {}", e),
    }

    // Adjust the path to the executable relative to the current working directory
    let relative_path = "OverBind.exe"; // Update this path accordingly
    let exe_path = match env::current_dir() {
        Ok(mut path) => {
            path.push(relative_path);
            path
        },
        Err(_) => return Err("Failed to get current directory".into()),
    };

    let output = Command::new(exe_path)
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|child| child.wait_with_output());

    match output {
        Ok(output) => {
            // Handle the output or result here if needed
            Ok("C++ executable ran successfully".into())
        }
        Err(e) => Err(format!("Failed to run C++ executable: {}", e)),
    }

}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![run_overbind])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}