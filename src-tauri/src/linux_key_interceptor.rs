#![cfg(target_os = "linux")]

use evdev::{Device, InputEventKind, Key};
use log::{debug, error, info, trace, warn};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use uinput::event::absolute::Hat::{X0, Y0};
use uinput::event::absolute::Position::{RX, RY, RZ, X, Y, Z};
use uinput::event::controller::DPad::{Down, Left, Right, Up};
use uinput::event::controller::GamePad::{
    East, Mode, North, Select, South, Start, ThumbL, ThumbR, West, TL, TR,
};
use uinput::event::Absolute::Hat;
use uinput::event::Absolute::Position;
use uinput::event::Controller::{DPad, GamePad};
use uinput::event::{Code, Kind, Press, Release};
use uinput::Device as UInputDevice;
use uinput::Event::{Absolute, Controller};
use x11rb::connection::Connection;
use x11rb::properties::WmClass;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask, Window,
};
use x11rb::protocol::Event;

use crate::key_interceptor::KeyInterceptorTrait;
use crate::text_masher::{text_masher, IS_MASHER_ACTIVE};
use crate::{get_config_path, Settings};

x11rb::atom_manager! {
    Atoms:
    AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_WM_VISIBLE_NAME,
        _NET_WM_NAME,
        WM_NAME,
        UTF8_STRING,
        STRING,
    }
}

trait Killable {
    fn kill(&mut self) -> std::io::Result<()>;
}

impl Killable for Child {
    fn kill(&mut self) -> std::io::Result<()> {
        std::process::Child::kill(self)
    }
}

impl Killable for Option<CommandChild> {
    fn kill(&mut self) -> std::io::Result<()> {
        if let Some(child) = self.take() {
            child
                .kill()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Child already killed",
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigJsonData {
    keycode: String,
    result_type: String,
    result_value: i32,
}

struct KeyState {
    is_pressed: bool,
    result_type: String,
    result_value: i32,
}

#[derive(Clone)]
struct OppositeKey {
    is_pressed: bool,
    is_virtual_pressed: bool,
    opposite_key_type: String,
    opposite_key_value: u16,
    opposite_key_mapping: Option<u16>,
}

static KEY_STATES: Lazy<Arc<RwLock<HashMap<u16, KeyState>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static OPPOSITE_KEY_STATES: Lazy<Arc<RwLock<HashMap<u16, OppositeKey>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static OPPOSITE_KEY_MAPPINGS: Lazy<Arc<RwLock<HashMap<u16, u16>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static DPAD_BUTTON_STATES: Lazy<Arc<RwLock<HashMap<u32, KeyState>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

//static IS_MASHER_ACTIVE: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

struct SharedState {
    uinput_controller: Option<UInputDevice>,
    uinput_keyboard: Option<UInputDevice>,
    allowed_programs: Option<Vec<String>>,
    device_path: Option<String>,
    active_app_name: Option<String>,
    is_cursor_overlay_enabled: bool,
    cursor_overlay_process: Option<Box<dyn Killable>>,
    block_kb_on_controller: bool,
}

unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

static SHARED_STATE: Lazy<Arc<RwLock<SharedState>>> = Lazy::new(|| {
    Arc::new(RwLock::new(SharedState {
        uinput_controller: None,
        uinput_keyboard: None,
        allowed_programs: None,
        device_path: None,
        active_app_name: None,
        is_cursor_overlay_enabled: false,
        cursor_overlay_process: None,
        block_kb_on_controller: false,
    }))
});

static SHOULD_RUN: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

pub(crate) struct LinuxKeyInterceptor {}

impl KeyInterceptorTrait for LinuxKeyInterceptor {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self {}
    }

    fn initialize(&mut self, settings: &Settings) -> Result<(), String> {
        // Create the virtual gamepad device
        let controller = uinput::default()
            .unwrap()
            .name("Overbind Virtual Gamepad")
            .unwrap()
            .event(Absolute(Position(X)))
            .unwrap()
            .min(-32768)
            .max(32767)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Position(Y)))
            .unwrap()
            .min(-32768)
            .max(32767)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Position(RX)))
            .unwrap()
            .min(-32768)
            .max(32767)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Position(RY)))
            .unwrap()
            .min(-32768)
            .max(32767)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Hat(X0)))
            .unwrap()
            .min(-1)
            .max(1)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Hat(Y0)))
            .unwrap()
            .min(-1)
            .max(1)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Position(Z)))
            .unwrap()
            .min(0)
            .max(1023)
            .fuzz(0)
            .flat(0)
            .event(Absolute(Position(RZ)))
            .unwrap()
            .min(0)
            .max(1023)
            .fuzz(0)
            .flat(0)
            .event(Controller(GamePad(North)))
            .unwrap()
            .event(Controller(GamePad(South)))
            .unwrap()
            .event(Controller(GamePad(East)))
            .unwrap()
            .event(Controller(GamePad(West)))
            .unwrap()
            .event(Controller(GamePad(TL)))
            .unwrap()
            .event(Controller(GamePad(TR)))
            .unwrap()
            .event(Controller(GamePad(ThumbL)))
            .unwrap()
            .event(Controller(GamePad(ThumbR)))
            .unwrap()
            .event(Controller(GamePad(Select)))
            .unwrap()
            .event(Controller(GamePad(Start)))
            .unwrap()
            .event(Controller(GamePad(Mode)))
            .unwrap()
            .create()
            .unwrap();

        let keyboard = uinput::default()
            .unwrap()
            .name("Overbind Virtual Keyboard")
            .unwrap()
            .event(uinput::event::Keyboard::All)
            .unwrap()
            .create()
            .unwrap();

        let mut shared_state = SHARED_STATE.write().unwrap();
        shared_state.uinput_controller = Some(controller);
        shared_state.uinput_keyboard = Some(keyboard);
        if !settings.allowed_programs.is_empty() {
            info!("Allowed programs: {:?}", settings.allowed_programs);
            shared_state.allowed_programs = Some(settings.allowed_programs.clone());
        }

        // Find the input device
        let mut device_name = "/dev/input/event0".to_owned();
        if settings.selected_input.is_some() {
            let symlink_path =
                Path::new("/dev/input/by-id").join(settings.selected_input.as_ref().unwrap());
            if symlink_path.exists() {
                if let Ok(input_path) = fs::read_link(symlink_path) {
                    if let Some(input_device) = input_path.file_name() {
                        info!("Found input device: {:?}", input_device);
                        device_name = format!("/dev/input/{}", input_device.to_str().unwrap());
                    }
                }
            } else {
                let backup_path =
                    Path::new("/dev/input/by-path").join(settings.selected_input.as_ref().unwrap());
                if backup_path.exists() {
                    if let Ok(input_path) = fs::read_link(backup_path) {
                        if let Some(input_device) = input_path.file_name() {
                            info!("Found input device: {:?}", input_device);
                            device_name = format!("/dev/input/{}", input_device.to_str().unwrap());
                        }
                    }
                }
            }
        }

        shared_state.device_path = Some(device_name);
        shared_state.is_cursor_overlay_enabled = settings.force_cursor;
        shared_state.block_kb_on_controller = settings.block_kb_on_controller;

        Ok(())
    }

    fn start(&mut self, app: &tauri::AppHandle) -> Result<(), String> {
        // Read keybindings from file
        let path = get_config_path()?;
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| e.to_string())?;

        let data: Vec<ConfigJsonData> =
            serde_json::from_str(&contents).map_err(|e| e.to_string())?;
        let mut key_states = HashMap::new();
        let mut opposite_key_states = HashMap::new();
        let mut opposite_key_mappings = HashMap::new();
        let mut dpad_button_states = HashMap::new();

        for item in &data {
            let windows_keycode =
                u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");
            let keycode = windows_code_to_evdev_enum(windows_keycode).unwrap().code();

            if item.result_type != "socd" {
                let mut result_value = item.result_value as i32;
                if item.result_type == "keyboard" || item.result_type == "mash_trigger" {
                    result_value = windows_code_to_evdev_enum(result_value as u32)
                        .unwrap()
                        .code() as i32;
                }
                if item.result_type == "thumb_lx"
                    || item.result_type == "thumb_ly"
                    || item.result_type == "thumb_rx"
                    || item.result_type == "thumb_ry"
                {
                    // result_value = result_value * 32767 / 30000;
                    if item.result_type == "thumb_ly" || item.result_type == "thumb_ry" {
                        result_value = -result_value;
                    }
                }
                if item.result_type == "trigger_l" || item.result_type == "trigger_r" {
                    result_value = result_value * 1023 / 255;
                }
                if item.result_type == "face_button" && item.result_value < 0x0010 {
                    let opposite_dpad =
                        dpad_button_opposite_key(item.result_value as u32).expect(&format!(
                            "Could not find opposite dpad button code: {}",
                            item.result_value
                        ));
                    dpad_button_states
                        .entry(item.result_value as u32)
                        .or_insert_with(|| KeyState {
                            is_pressed: false,
                            result_type: item.result_type.clone(),
                            result_value: opposite_dpad as i32,
                        });
                    debug!(
                        "DPad button code: {:?}, Opposite dpad button code: {:?}",
                        item.result_value, opposite_dpad
                    )
                }

                let key_state = key_states.entry(keycode).or_insert_with(|| KeyState {
                    is_pressed: false,
                    result_type: item.result_type.clone(),
                    result_value: result_value,
                });
                debug!(
                    "Keycode: {:?}, ResultType: {:?}, ResultValue {:?}",
                    keycode, key_state.result_type, key_state.result_value
                );
            }
        }

        for item in &data {
            let windows_keycode =
                u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");
            let keycode = windows_code_to_evdev_enum(windows_keycode).unwrap().code();

            if item.result_type == "socd" {
                let windows_opposite_keycode = item.result_value as u16;
                let opposite_keycode = windows_code_to_evdev_enum(windows_opposite_keycode.into())
                    .unwrap()
                    .code();

                let this_key_state_mapping = key_states.get(&keycode.clone());
                let opposite_key_state_mapping = key_states.get(&opposite_keycode.clone());
                let mut key_type = String::from("keyboard");
                let mut opposite_key_mapping = None;

                if this_key_state_mapping.is_some()
                    && (this_key_state_mapping.unwrap().result_type == "keyboard")
                {
                    opposite_key_mappings.insert(
                        keycode.clone(),
                        this_key_state_mapping.unwrap().result_value as u16,
                    );
                } else {
                    opposite_key_mappings.insert(keycode.clone(), keycode.clone());
                }

                if opposite_key_state_mapping.is_some()
                    && (opposite_key_state_mapping.unwrap().result_type == "keyboard"
                        || opposite_key_state_mapping.unwrap().result_type == "face_button")
                {
                    key_type = opposite_key_state_mapping.unwrap().result_type.clone();
                    opposite_key_mapping =
                        Some(opposite_key_state_mapping.unwrap().result_value as u16);
                }

                let opposite_key_state = opposite_key_states
                    .entry(opposite_key_mappings.get(&keycode).unwrap().clone())
                    .or_insert_with(|| OppositeKey {
                        is_pressed: false,
                        is_virtual_pressed: false,
                        opposite_key_value: opposite_keycode,
                        opposite_key_type: key_type,
                        opposite_key_mapping,
                    });

                debug!(
                    "Keycode: {:?}, KeycodeMapping: {:?}, OppositeKeycode: {:?}, OppositeKeyMapping: {:?}",
                    keycode,
                    opposite_key_mappings.get(&keycode).unwrap(),
                    opposite_key_state.opposite_key_value,
                    opposite_key_state.opposite_key_mapping
                );
            }
        }

        *KEY_STATES.write().unwrap() = key_states;
        *OPPOSITE_KEY_STATES.write().unwrap() = opposite_key_states;
        *OPPOSITE_KEY_MAPPINGS.write().unwrap() = opposite_key_mappings;
        *DPAD_BUTTON_STATES.write().unwrap() = dpad_button_states;

        SHOULD_RUN.store(true, Ordering::SeqCst);

        thread::spawn(|| {
            text_masher(|pressed| {
                for (_, key_state) in KEY_STATES.read().unwrap().iter() {
                    if key_state.result_type == "mash_trigger" {
                        send_keyboard_event(key_state.result_value as u16, pressed);
                    }
                }
                sync_keyboard();
            });
        });

        //Thread to update the active application name asynchronously using X11 events
        thread::spawn(move || {
            let (conn, screen_num) = x11rb::connect(None).unwrap();
            let screen = &conn.setup().roots[screen_num];

            let atoms = Atoms::new(&conn).unwrap().reply().unwrap();

            let vau = ChangeWindowAttributesAux::default().event_mask(EventMask::PROPERTY_CHANGE);
            conn.change_window_attributes(screen.root, &vau).unwrap();

            // Without this, the change_window_attributes() is not actually sent to the X11 server
            conn.flush().unwrap();

            check_focus(&conn, &atoms, screen.root);
            while SHOULD_RUN.load(Ordering::SeqCst) {
                match conn.wait_for_event().unwrap() {
                    Event::PropertyNotify(event) if event.atom == atoms._NET_ACTIVE_WINDOW => {
                        let window_name = check_focus(&conn, &atoms, screen.root);
                        {
                            let mut shared_state = SHARED_STATE.write().unwrap();
                            shared_state.active_app_name = Some(window_name);
                            info!("Active app name: {:?}", shared_state.active_app_name);

                            if shared_state.is_cursor_overlay_enabled {
                                match UnixStream::connect("/tmp/cursor_overlay.sock") {
                                    Ok(mut stream) => {
                                        let mut command = None;
                                        if shared_state.allowed_programs.as_ref().is_none()
                                            || (shared_state
                                                .allowed_programs
                                                .as_ref()
                                                .unwrap()
                                                .contains(
                                                    &shared_state.active_app_name.as_ref().unwrap(),
                                                ))
                                        {
                                            debug!("Showing cursor");
                                            command = Some("show");
                                        } else {
                                            debug!("Hiding cursor");
                                            command = Some("hide");
                                        }

                                        stream
                                            .write_all(command.unwrap().as_bytes())
                                            .expect("Failed to send command");
                                    }
                                    Err(e) => {
                                        debug!("Failed to connect to cursor overlay. You may have to delete the socket file at /tmp/cursor_overlay.sock and restart OverBind. Error: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        // Start listening for key events
        info!("Spawning key event listener thread");
        thread::spawn(move || {
            let mut allowed_programs = Option::None;
            let mut device_path = String::from("/dev/input/event0"); // Default in case we didn't find the device
            {
                let shared_state = SHARED_STATE.read().unwrap();
                if let Some(ref path) = shared_state.device_path {
                    device_path = path.clone();
                }
                if let Some(ref programs) = shared_state.allowed_programs {
                    allowed_programs = Some(programs.clone());
                }
            }
            let mut device = Device::open(&device_path).unwrap();
            info!("Opened device: {:?}", device_path);

            device.grab().unwrap();

            loop {
                for event in device.fetch_events().expect("Failed to fetch events") {
                    match event.kind() {
                        InputEventKind::Key(key_event) => {
                            let handle_start = Instant::now();
                            let mut active_app_name = None;
                            {
                                let shared_state = SHARED_STATE.read().unwrap();
                                active_app_name = shared_state.active_app_name.clone();
                            }
                            if SHOULD_RUN.load(Ordering::SeqCst) {
                                if allowed_programs.as_ref().is_none()
                                    || (active_app_name.is_some()
                                        && allowed_programs
                                            .as_ref()
                                            .unwrap()
                                            .contains(&active_app_name.unwrap()))
                                {
                                    handle_key_event(key_event.code(), event.value() != 0);
                                } else {
                                    send_keyboard_event(key_event.code(), event.value() != 0);
                                    sync_keyboard();
                                }
                                let handle_duration = handle_start.elapsed();
                                debug!("Handle duration in us: {:?}", handle_duration.as_micros());
                            } else {
                                // Because fetch_events is blocking when overbind is stopped we still will process one more event
                                // Send it and immediately release it to resent the virtual keyboard back to normal
                                send_keyboard_event(key_event.code(), event.value() != 0);
                                send_keyboard_event(key_event.code(), false);
                                sync_keyboard();
                                break;
                            }
                        }
                        _ => (),
                    }
                }
                if !SHOULD_RUN.load(Ordering::SeqCst) {
                    break;
                }
            }

            info!("Ungrabbing device");
            device.ungrab().unwrap();
        });

        // Cursor overlay
        {
            let mut shared_state = SHARED_STATE.write().unwrap();
            if shared_state.is_cursor_overlay_enabled {
                let child: Box<dyn Killable> = if tauri::is_dev() {
                    debug!("Starting cursor overlay in dev");
                    Box::new(
                        Command::new("cargo")
                            .arg("run")
                            .arg("--bin")
                            .arg("cursor-overlay-x86_64-unknown-linux-gnu")
                            .spawn()
                            .expect("Failed to start cursor overlay"),
                    )
                } else {
                    debug!("Starting cursor overlay in prod");
                    let sidecar_command = app.shell().sidecar("cursor-overlay").unwrap();
                    let (mut rx, child) = sidecar_command
                        .spawn()
                        .expect("Failed to start cursor overlay");

                    tauri::async_runtime::spawn(async move {
                        // read events such as stdout
                        while let Some(event) = rx.recv().await {
                            match event {
                                CommandEvent::Stdout(data) => {
                                    debug!(
                                        "cursor-overlay stdout: {}",
                                        String::from_utf8(data).unwrap_or("Unknown".to_string())
                                    );
                                }
                                CommandEvent::Stderr(data) => {
                                    error!(
                                        "cursor-overlay stderr: {}",
                                        String::from_utf8(data).unwrap_or("Unknown".to_string())
                                    );
                                }
                                CommandEvent::Terminated(code) => {
                                    error!(
                                        "cursor-overlay exited with code: {}",
                                        code.code.unwrap_or(1337).to_string()
                                    );
                                }
                                CommandEvent::Error(data) => {
                                    error!("cursor-overlay error: {}", data)
                                }
                                _ => {}
                            }
                        }
                    });

                    Box::new(Some(child))
                };

                shared_state.cursor_overlay_process = Some(child);
            }
        }

        Ok(())
    }

    fn stop(&self, _app: &tauri::AppHandle) {
        SHOULD_RUN.store(false, Ordering::SeqCst);

        {
            let mut shared_state = SHARED_STATE.write().unwrap();
            if let Some(child) = shared_state.cursor_overlay_process.as_mut() {
                child.kill().expect("Failed to stop cursor overlay");
            }
        }
    }

    fn is_running(&self) -> bool {
        SHOULD_RUN.load(Ordering::SeqCst)
    }
}

fn handle_key_event(key_code: u16, key_is_down: bool) {
    // Update Key State
    {
        let mut key_states = KEY_STATES.write().unwrap();
        match key_states.get_mut(&key_code) {
            Some(state) => state.is_pressed = key_is_down,
            _ => (),
        }
    }
    debug!("Keycode: {:?}, Key is down: {:?}", key_code, key_is_down);

    // SOCD
    {
        let mut opposite_key_states = OPPOSITE_KEY_STATES.write().unwrap();
        let opposite_key_mappings = OPPOSITE_KEY_MAPPINGS.write().unwrap();

        let cloned_key_state;
        if opposite_key_mappings.contains_key(&key_code) {
            {
                let mapped_key = opposite_key_mappings.get(&key_code).unwrap();
                let key_state = opposite_key_states.get_mut(&mapped_key).unwrap();
                key_state.is_pressed = key_is_down;
                key_state.is_virtual_pressed = key_is_down;

                cloned_key_state = key_state.clone();
            }

            let opposite_key_value = cloned_key_state.opposite_key_value;
            let mapping_opposite_key_state =
                cloned_key_state
                    .opposite_key_mapping
                    .and_then(|opposite_key_mapping| {
                        opposite_key_states.get_mut(&opposite_key_mapping)
                    });
            let opposite_key_state = match mapping_opposite_key_state {
                Some(value) => value,
                None => opposite_key_states.get_mut(&opposite_key_value).unwrap(),
            };

            if key_is_down && opposite_key_state.is_pressed && opposite_key_state.is_virtual_pressed
            {
                opposite_key_state.is_virtual_pressed = false;
                send_keyboard_event(cloned_key_state.opposite_key_value, false);
            } else if !key_is_down && opposite_key_state.is_pressed {
                opposite_key_state.is_virtual_pressed = true;
                send_keyboard_event(cloned_key_state.opposite_key_value, true);
            }
        }
    }

    // Mash Trigger State Update
    {
        let key_states = KEY_STATES.read().unwrap();
        if matches!(key_states.get(&key_code), Some(state) if state.result_type == "mash_trigger") {
            let mut is_masher_active = true;
            for (_, key_state) in key_states
                .iter()
                .filter(|&(_, ks)| ks.result_type == "mash_trigger")
            {
                if !key_state.is_pressed {
                    is_masher_active = false;
                    break;
                }
            }

            if IS_MASHER_ACTIVE.load(Ordering::SeqCst) != is_masher_active {
                IS_MASHER_ACTIVE.store(is_masher_active, Ordering::SeqCst);
            }
        }
    }

    // Rebinds
    {
        let key_states = KEY_STATES.read().unwrap();
        if let Some(key_state) = key_states.get(&key_code) {
            if key_state.result_type == "keyboard" || key_state.result_type == "mash_trigger" {
                send_keyboard_event(key_state.result_value as u16, key_is_down);
                sync_keyboard();
                return;
            } else if key_state.result_type == "face_button" {
                if key_state.result_value as u32 <= 0x0010 {
                    let event_key = dpad_button_to_abs_key(key_state.result_value as u32).expect(
                        &format!("Invalid dpad button code: {}", key_state.result_value),
                    );
                    let event_value = dpad_button_to_abs_value(key_state.result_value as u32)
                        .expect(&format!(
                            "Invalid dpad button code: {}",
                            key_state.result_value
                        ));

                    if key_is_down {
                        debug!(
                            "Sending hat event (dpad down): {:?}, {:?}",
                            event_key, event_value
                        );
                        send_hat_event(event_key, event_value, true);
                    } else {
                        let dpad_states = DPAD_BUTTON_STATES.read().unwrap();
                        debug!(
                            "Searching opposite dpad state for {:?}",
                            key_state.result_value
                        );
                        // Print all dpad states
                        for (key, value) in dpad_states.iter() {
                            debug!("Key: {:?}, Value: {:?}", key, value.result_value);
                        }
                        if let Some(opposite_dpad_state) =
                            dpad_states.get(&(key_state.result_value as u32))
                        {
                            if opposite_dpad_state.is_pressed {
                                debug!(
                                    "Sending hat even (opposite dpad down): {:?}, {:?}",
                                    event_key,
                                    dpad_button_to_abs_value(
                                        opposite_dpad_state.result_value as u32
                                    )
                                    .expect(&format!(
                                        "Invalid dpad button code: {}",
                                        opposite_dpad_state.result_value
                                    ))
                                );
                                send_hat_event(
                                    event_key,
                                    dpad_button_to_abs_value(
                                        opposite_dpad_state.result_value as u32,
                                    )
                                    .expect(&format!(
                                        "Invalid dpad button code: {}",
                                        opposite_dpad_state.result_value
                                    )),
                                    true,
                                );
                            } else {
                                debug!(
                                    "Sending hat event (opposite of {:?} not pressed): {:?}, {:?}",
                                    key_state.result_value, event_key, event_value
                                );
                                send_hat_event(event_key, event_value, false);
                            }
                        } else {
                            debug!(
                                "Sending hat event (opposite of {:?} not found): {:?}, {:?}",
                                key_state.result_value, event_key, event_value
                            );
                            send_hat_event(event_key, event_value, false);
                        }
                    }

                    let mut dpad_states = DPAD_BUTTON_STATES.write().unwrap();
                    if let Some(dpad_state) = dpad_states
                        .iter_mut()
                        .find(|(_, state)| state.result_value == key_state.result_value)
                    {
                        dpad_state.1.is_pressed = key_is_down;
                        debug!(
                            "Setting dpad state for {:?} to {:?}",
                            key_state.result_value, key_is_down
                        );
                    }
                } else {
                    let event = face_button_to_uinput_enum(key_state.result_value as u32).expect(
                        &format!("Invalid controller code: {}", key_state.result_value),
                    );

                    send_face_button_event(event, key_is_down);
                }
            } else if key_state.result_type != "socd" {
                send_position_event(
                    position_to_uinput_enum(&key_state.result_type)
                        .expect(&format!("Invalid thumb code: {}", key_state.result_value)),
                    key_state.result_value as i32,
                    key_is_down,
                );
            }
        } else {
            send_keyboard_event(key_code, key_is_down);
            sync_keyboard();
            return;
        }
    }

    // Controller SOCD
    {
        let opposite_key_states = OPPOSITE_KEY_STATES.read().unwrap();
        for (_, opposite_key_state) in opposite_key_states
            .iter()
            .filter(|&(_, ks)| ks.opposite_key_type == String::from("face_button"))
        {
            if let Some(opposite_key_mapping) = opposite_key_state.opposite_key_mapping {
                let mapped_key = opposite_key_mapping as u16;
                send_face_button_event(
                    face_button_to_uinput_enum(mapped_key as u32)
                        .expect(&format!("Invalid face button code: {}", mapped_key)),
                    opposite_key_state.is_virtual_pressed,
                );
            }
        }
    }

    if !SHARED_STATE.read().unwrap().block_kb_on_controller {
        send_keyboard_event(key_code, key_is_down);
    }

    sync_keyboard();
    sync_controller();
}

fn send_keyboard_event(key_code: u16, key_is_down: bool) {
    let mut shared_state = SHARED_STATE.write().unwrap();
    if key_is_down {
        shared_state
            .uinput_keyboard
            .as_mut()
            .unwrap()
            .press(&evdev_enum_to_uinput_enum(key_code).unwrap())
            .unwrap();
    } else {
        shared_state
            .uinput_keyboard
            .as_mut()
            .unwrap()
            .release(&evdev_enum_to_uinput_enum(key_code).unwrap())
            .unwrap();
    }
}

fn send_face_button_event(button: UInputOutput, key_is_down: bool) {
    let mut shared_state = SHARED_STATE.write().unwrap();
    if key_is_down {
        shared_state
            .uinput_controller
            .as_mut()
            .unwrap()
            .press(&button)
            .unwrap();
    } else {
        shared_state
            .uinput_controller
            .as_mut()
            .unwrap()
            .release(&button)
            .unwrap();
    }
}

fn send_position_event(event: uinput::event::absolute::Position, value: i32, key_is_down: bool) {
    let mut shared_state = SHARED_STATE.write().unwrap();
    if key_is_down {
        shared_state
            .uinput_controller
            .as_mut()
            .unwrap()
            .position(&event, value)
            .unwrap();
    } else {
        shared_state
            .uinput_controller
            .as_mut()
            .unwrap()
            .position(&event, 0)
            .unwrap();
    }
}

fn send_hat_event(event: uinput::event::absolute::Hat, value: i32, key_is_down: bool) {
    let mut shared_state = SHARED_STATE.write().unwrap();
    if key_is_down {
        shared_state
            .uinput_controller
            .as_mut()
            .unwrap()
            .position(&event, value)
            .unwrap();
    } else {
        shared_state
            .uinput_controller
            .as_mut()
            .unwrap()
            .position(&event, 0)
            .unwrap();
    }
}

fn sync_keyboard() {
    let mut shared_state = SHARED_STATE.write().unwrap();
    shared_state
        .uinput_keyboard
        .as_mut()
        .unwrap()
        .synchronize()
        .unwrap();
}

fn sync_controller() {
    let mut shared_state = SHARED_STATE.write().unwrap();
    shared_state
        .uinput_controller
        .as_mut()
        .unwrap()
        .synchronize()
        .unwrap();
}

fn windows_code_to_evdev_enum(code: u32) -> Option<Key> {
    match code {
        0x08 => Some(Key::KEY_BACKSPACE),
        0x09 => Some(Key::KEY_TAB),
        0x0C => Some(Key::KEY_CLEAR),
        0x0D => Some(Key::KEY_ENTER),
        0x10 => Some(Key::KEY_LEFTSHIFT),
        0x11 => Some(Key::KEY_LEFTCTRL),
        0x12 => Some(Key::KEY_LEFTALT),
        0x13 => Some(Key::KEY_PAUSE),
        0x14 => Some(Key::KEY_CAPSLOCK),
        0x1B => Some(Key::KEY_ESC),
        0x20 => Some(Key::KEY_SPACE),
        0x21 => Some(Key::KEY_PAGEUP),
        0x22 => Some(Key::KEY_PAGEDOWN),
        0x23 => Some(Key::KEY_END),
        0x24 => Some(Key::KEY_HOME),
        0x25 => Some(Key::KEY_LEFT),
        0x26 => Some(Key::KEY_UP),
        0x27 => Some(Key::KEY_RIGHT),
        0x28 => Some(Key::KEY_DOWN),
        0x29 => Some(Key::KEY_SELECT),
        0x2A => Some(Key::KEY_PRINT),
        0x2D => Some(Key::KEY_INSERT),
        0x2E => Some(Key::KEY_DELETE),
        0x2F => Some(Key::KEY_HELP),
        0x30 => Some(Key::KEY_0),
        0x31 => Some(Key::KEY_1),
        0x32 => Some(Key::KEY_2),
        0x33 => Some(Key::KEY_3),
        0x34 => Some(Key::KEY_4),
        0x35 => Some(Key::KEY_5),
        0x36 => Some(Key::KEY_6),
        0x37 => Some(Key::KEY_7),
        0x38 => Some(Key::KEY_8),
        0x39 => Some(Key::KEY_9),
        0x41 => Some(Key::KEY_A),
        0x42 => Some(Key::KEY_B),
        0x43 => Some(Key::KEY_C),
        0x44 => Some(Key::KEY_D),
        0x45 => Some(Key::KEY_E),
        0x46 => Some(Key::KEY_F),
        0x47 => Some(Key::KEY_G),
        0x48 => Some(Key::KEY_H),
        0x49 => Some(Key::KEY_I),
        0x4A => Some(Key::KEY_J),
        0x4B => Some(Key::KEY_K),
        0x4C => Some(Key::KEY_L),
        0x4D => Some(Key::KEY_M),
        0x4E => Some(Key::KEY_N),
        0x4F => Some(Key::KEY_O),
        0x50 => Some(Key::KEY_P),
        0x51 => Some(Key::KEY_Q),
        0x52 => Some(Key::KEY_R),
        0x53 => Some(Key::KEY_S),
        0x54 => Some(Key::KEY_T),
        0x55 => Some(Key::KEY_U),
        0x56 => Some(Key::KEY_V),
        0x57 => Some(Key::KEY_W),
        0x58 => Some(Key::KEY_X),
        0x59 => Some(Key::KEY_Y),
        0x5A => Some(Key::KEY_Z),
        0x5B => Some(Key::KEY_LEFTMETA),
        0x5C => Some(Key::KEY_RIGHTMETA),
        0x5D => Some(Key::KEY_APPSELECT),
        0x5F => Some(Key::KEY_SLEEP),
        0x60 => Some(Key::KEY_KP0),
        0x61 => Some(Key::KEY_KP1),
        0x62 => Some(Key::KEY_KP2),
        0x63 => Some(Key::KEY_KP3),
        0x64 => Some(Key::KEY_KP4),
        0x65 => Some(Key::KEY_KP5),
        0x66 => Some(Key::KEY_KP6),
        0x67 => Some(Key::KEY_KP7),
        0x68 => Some(Key::KEY_KP8),
        0x69 => Some(Key::KEY_KP9),
        0x6A => Some(Key::KEY_KPASTERISK),
        0x6B => Some(Key::KEY_KPPLUS),
        0x6C => Some(Key::KEY_KPCOMMA),
        0x6D => Some(Key::KEY_KPMINUS),
        0x6E => Some(Key::KEY_KPDOT),
        0x6F => Some(Key::KEY_KPSLASH),
        0x70 => Some(Key::KEY_F1),
        0x71 => Some(Key::KEY_F2),
        0x72 => Some(Key::KEY_F3),
        0x73 => Some(Key::KEY_F4),
        0x74 => Some(Key::KEY_F5),
        0x75 => Some(Key::KEY_F6),
        0x76 => Some(Key::KEY_F7),
        0x77 => Some(Key::KEY_F8),
        0x78 => Some(Key::KEY_F9),
        0x79 => Some(Key::KEY_F10),
        0x7A => Some(Key::KEY_F11),
        0x7B => Some(Key::KEY_F12),
        0x7C => Some(Key::KEY_F13),
        0x7D => Some(Key::KEY_F14),
        0x7E => Some(Key::KEY_F15),
        0x7F => Some(Key::KEY_F16),
        0x80 => Some(Key::KEY_F17),
        0x81 => Some(Key::KEY_F18),
        0x82 => Some(Key::KEY_F19),
        0x83 => Some(Key::KEY_F20),
        0x84 => Some(Key::KEY_F21),
        0x85 => Some(Key::KEY_F22),
        0x86 => Some(Key::KEY_F23),
        0x87 => Some(Key::KEY_F24),
        0x90 => Some(Key::KEY_NUMLOCK),
        0x91 => Some(Key::KEY_SCROLLLOCK),
        0xA0 => Some(Key::KEY_LEFTSHIFT),
        0xA1 => Some(Key::KEY_RIGHTSHIFT),
        0xA2 => Some(Key::KEY_LEFTCTRL),
        0xA3 => Some(Key::KEY_RIGHTCTRL),
        0xA4 => Some(Key::KEY_LEFTALT),
        0xA5 => Some(Key::KEY_RIGHTALT),
        0xA6 => Some(Key::KEY_BACK),
        0xA7 => Some(Key::KEY_FORWARD),
        0xA8 => Some(Key::KEY_REFRESH),
        0xA9 => Some(Key::KEY_STOP),
        0xAA => Some(Key::KEY_SEARCH),
        0xAB => Some(Key::KEY_FAVORITES),
        0xAC => Some(Key::KEY_HOMEPAGE),
        0xAD => Some(Key::KEY_MUTE),
        0xAE => Some(Key::KEY_VOLUMEDOWN),
        0xAF => Some(Key::KEY_VOLUMEUP),
        0xB0 => Some(Key::KEY_NEXT),
        0xB1 => Some(Key::KEY_PREVIOUS),
        0xB2 => Some(Key::KEY_STOP),
        0xB3 => Some(Key::KEY_PLAYPAUSE),
        0xB4 => Some(Key::KEY_MAIL),
        0xB5 => Some(Key::KEY_MEDIA),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum UInputOutput {
    Key(uinput::event::keyboard::Key),
    KeyPad(uinput::event::keyboard::KeyPad),
    GamePad(uinput::event::controller::GamePad),
}

impl Press for UInputOutput {}
impl Release for UInputOutput {}
impl Code for UInputOutput {
    fn code(&self) -> i32 {
        match self {
            UInputOutput::Key(key) => key.code(),
            UInputOutput::KeyPad(key_pad) => key_pad.code(),
            UInputOutput::GamePad(game_pad) => game_pad.code(),
        }
    }
}

impl Kind for UInputOutput {
    fn kind(&self) -> i32 {
        match self {
            UInputOutput::Key(key) => key.kind(),
            UInputOutput::KeyPad(key_pad) => key_pad.kind(),
            UInputOutput::GamePad(game_pad) => game_pad.kind(),
        }
    }
}

fn evdev_enum_to_uinput_enum(code: u16) -> Option<UInputOutput> {
    let key = Key(code);
    match key {
        Key::KEY_RESERVED => Some(UInputOutput::Key(uinput::event::keyboard::Key::Reserved)),
        Key::KEY_ESC => Some(UInputOutput::Key(uinput::event::keyboard::Key::Esc)),
        Key::KEY_1 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_1)),
        Key::KEY_2 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_2)),
        Key::KEY_3 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_3)),
        Key::KEY_4 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_4)),
        Key::KEY_5 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_5)),
        Key::KEY_6 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_6)),
        Key::KEY_7 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_7)),
        Key::KEY_8 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_8)),
        Key::KEY_9 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_9)),
        Key::KEY_0 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_0)),
        Key::KEY_MINUS => Some(UInputOutput::Key(uinput::event::keyboard::Key::Minus)),
        Key::KEY_EQUAL => Some(UInputOutput::Key(uinput::event::keyboard::Key::Equal)),
        Key::KEY_BACKSPACE => Some(UInputOutput::Key(uinput::event::keyboard::Key::BackSpace)),
        Key::KEY_TAB => Some(UInputOutput::Key(uinput::event::keyboard::Key::Tab)),
        Key::KEY_Q => Some(UInputOutput::Key(uinput::event::keyboard::Key::Q)),
        Key::KEY_W => Some(UInputOutput::Key(uinput::event::keyboard::Key::W)),
        Key::KEY_E => Some(UInputOutput::Key(uinput::event::keyboard::Key::E)),
        Key::KEY_R => Some(UInputOutput::Key(uinput::event::keyboard::Key::R)),
        Key::KEY_T => Some(UInputOutput::Key(uinput::event::keyboard::Key::T)),
        Key::KEY_Y => Some(UInputOutput::Key(uinput::event::keyboard::Key::Y)),
        Key::KEY_U => Some(UInputOutput::Key(uinput::event::keyboard::Key::U)),
        Key::KEY_I => Some(UInputOutput::Key(uinput::event::keyboard::Key::I)),
        Key::KEY_O => Some(UInputOutput::Key(uinput::event::keyboard::Key::O)),
        Key::KEY_P => Some(UInputOutput::Key(uinput::event::keyboard::Key::P)),
        Key::KEY_LEFTBRACE => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftBrace)),
        Key::KEY_RIGHTBRACE => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightBrace)),
        Key::KEY_ENTER => Some(UInputOutput::Key(uinput::event::keyboard::Key::Enter)),
        Key::KEY_LEFTCTRL => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftControl)),
        Key::KEY_A => Some(UInputOutput::Key(uinput::event::keyboard::Key::A)),
        Key::KEY_S => Some(UInputOutput::Key(uinput::event::keyboard::Key::S)),
        Key::KEY_D => Some(UInputOutput::Key(uinput::event::keyboard::Key::D)),
        Key::KEY_F => Some(UInputOutput::Key(uinput::event::keyboard::Key::F)),
        Key::KEY_G => Some(UInputOutput::Key(uinput::event::keyboard::Key::G)),
        Key::KEY_H => Some(UInputOutput::Key(uinput::event::keyboard::Key::H)),
        Key::KEY_J => Some(UInputOutput::Key(uinput::event::keyboard::Key::J)),
        Key::KEY_K => Some(UInputOutput::Key(uinput::event::keyboard::Key::K)),
        Key::KEY_L => Some(UInputOutput::Key(uinput::event::keyboard::Key::L)),
        Key::KEY_SEMICOLON => Some(UInputOutput::Key(uinput::event::keyboard::Key::SemiColon)),
        Key::KEY_APOSTROPHE => Some(UInputOutput::Key(uinput::event::keyboard::Key::Apostrophe)),
        Key::KEY_GRAVE => Some(UInputOutput::Key(uinput::event::keyboard::Key::Grave)),
        Key::KEY_LEFTSHIFT => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftShift)),
        Key::KEY_BACKSLASH => Some(UInputOutput::Key(uinput::event::keyboard::Key::BackSlash)),
        Key::KEY_Z => Some(UInputOutput::Key(uinput::event::keyboard::Key::Z)),
        Key::KEY_X => Some(UInputOutput::Key(uinput::event::keyboard::Key::X)),
        Key::KEY_C => Some(UInputOutput::Key(uinput::event::keyboard::Key::C)),
        Key::KEY_V => Some(UInputOutput::Key(uinput::event::keyboard::Key::V)),
        Key::KEY_B => Some(UInputOutput::Key(uinput::event::keyboard::Key::B)),
        Key::KEY_N => Some(UInputOutput::Key(uinput::event::keyboard::Key::N)),
        Key::KEY_M => Some(UInputOutput::Key(uinput::event::keyboard::Key::M)),
        Key::KEY_COMMA => Some(UInputOutput::Key(uinput::event::keyboard::Key::Comma)),
        Key::KEY_DOT => Some(UInputOutput::Key(uinput::event::keyboard::Key::Dot)),
        Key::KEY_SLASH => Some(UInputOutput::Key(uinput::event::keyboard::Key::Slash)),
        Key::KEY_RIGHTSHIFT => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightShift)),
        Key::KEY_KPASTERISK => Some(UInputOutput::KeyPad(
            uinput::event::keyboard::KeyPad::Asterisk,
        )),
        Key::KEY_LEFTALT => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftAlt)),
        Key::KEY_SPACE => Some(UInputOutput::Key(uinput::event::keyboard::Key::Space)),
        Key::KEY_CAPSLOCK => Some(UInputOutput::Key(uinput::event::keyboard::Key::CapsLock)),
        Key::KEY_F1 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F1)),
        Key::KEY_F2 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F2)),
        Key::KEY_F3 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F3)),
        Key::KEY_F4 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F4)),
        Key::KEY_F5 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F5)),
        Key::KEY_F6 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F6)),
        Key::KEY_F7 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F7)),
        Key::KEY_F8 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F8)),
        Key::KEY_F9 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F9)),
        Key::KEY_F10 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F10)),
        Key::KEY_NUMLOCK => Some(UInputOutput::Key(uinput::event::keyboard::Key::NumLock)),
        Key::KEY_SCROLLLOCK => Some(UInputOutput::Key(uinput::event::keyboard::Key::ScrollLock)),
        Key::KEY_KP7 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_7)),
        Key::KEY_KP8 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_8)),
        Key::KEY_KP9 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_9)),
        Key::KEY_KPMINUS => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Minus)),
        Key::KEY_KP4 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_4)),
        Key::KEY_KP5 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_5)),
        Key::KEY_KP6 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_6)),
        Key::KEY_KPPLUS => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Plus)),
        Key::KEY_KP1 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_1)),
        Key::KEY_KP2 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_2)),
        Key::KEY_KP3 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_3)),
        Key::KEY_KP0 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_0)),
        Key::KEY_KPDOT => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Dot)),
        Key::KEY_KPENTER => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Enter)),
        Key::KEY_RIGHTCTRL => Some(UInputOutput::Key(
            uinput::event::keyboard::Key::RightControl,
        )),
        Key::KEY_KPSLASH => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Slash)),
        Key::KEY_RIGHTALT => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightAlt)),
        Key::KEY_HOME => Some(UInputOutput::Key(uinput::event::keyboard::Key::Home)),
        Key::KEY_UP => Some(UInputOutput::Key(uinput::event::keyboard::Key::Up)),
        Key::KEY_PAGEUP => Some(UInputOutput::Key(uinput::event::keyboard::Key::PageUp)),
        Key::KEY_LEFT => Some(UInputOutput::Key(uinput::event::keyboard::Key::Left)),
        Key::KEY_RIGHT => Some(UInputOutput::Key(uinput::event::keyboard::Key::Right)),
        Key::KEY_END => Some(UInputOutput::Key(uinput::event::keyboard::Key::End)),
        Key::KEY_DOWN => Some(UInputOutput::Key(uinput::event::keyboard::Key::Down)),
        Key::KEY_PAGEDOWN => Some(UInputOutput::Key(uinput::event::keyboard::Key::PageDown)),
        Key::KEY_INSERT => Some(UInputOutput::Key(uinput::event::keyboard::Key::Insert)),
        Key::KEY_DELETE => Some(UInputOutput::Key(uinput::event::keyboard::Key::Delete)),
        Key::KEY_KPEQUAL => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Equal)),
        Key::KEY_KPPLUSMINUS => Some(UInputOutput::KeyPad(
            uinput::event::keyboard::KeyPad::PlusMinus,
        )),
        Key::KEY_KPCOMMA => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Comma)),
        Key::KEY_LEFTMETA => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftMeta)),
        Key::KEY_RIGHTMETA => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightMeta)),
        _ => None,
    }
}

fn windows_code_to_uinput_enum(code: u32) -> Option<UInputOutput> {
    match code {
        0x08 => Some(UInputOutput::Key(uinput::event::keyboard::Key::BackSpace)),
        0x09 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Tab)),
        0x0D => Some(UInputOutput::Key(uinput::event::keyboard::Key::Enter)),
        0x10 => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftShift)),
        0x11 => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftControl)),
        0x12 => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftAlt)),
        0x14 => Some(UInputOutput::Key(uinput::event::keyboard::Key::CapsLock)),
        0x1B => Some(UInputOutput::Key(uinput::event::keyboard::Key::Esc)),
        0x20 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Space)),
        0x21 => Some(UInputOutput::Key(uinput::event::keyboard::Key::PageUp)),
        0x22 => Some(UInputOutput::Key(uinput::event::keyboard::Key::PageDown)),
        0x23 => Some(UInputOutput::Key(uinput::event::keyboard::Key::End)),
        0x24 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Home)),
        0x25 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Left)),
        0x26 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Up)),
        0x27 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Right)),
        0x28 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Down)),
        0x2D => Some(UInputOutput::Key(uinput::event::keyboard::Key::Insert)),
        0x2E => Some(UInputOutput::Key(uinput::event::keyboard::Key::Delete)),
        0x30 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_0)),
        0x31 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_1)),
        0x32 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_2)),
        0x33 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_3)),
        0x34 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_4)),
        0x35 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_5)),
        0x36 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_6)),
        0x37 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_7)),
        0x38 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_8)),
        0x39 => Some(UInputOutput::Key(uinput::event::keyboard::Key::_9)),
        0x41 => Some(UInputOutput::Key(uinput::event::keyboard::Key::A)),
        0x42 => Some(UInputOutput::Key(uinput::event::keyboard::Key::B)),
        0x43 => Some(UInputOutput::Key(uinput::event::keyboard::Key::C)),
        0x44 => Some(UInputOutput::Key(uinput::event::keyboard::Key::D)),
        0x45 => Some(UInputOutput::Key(uinput::event::keyboard::Key::E)),
        0x46 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F)),
        0x47 => Some(UInputOutput::Key(uinput::event::keyboard::Key::G)),
        0x48 => Some(UInputOutput::Key(uinput::event::keyboard::Key::H)),
        0x49 => Some(UInputOutput::Key(uinput::event::keyboard::Key::I)),
        0x4A => Some(UInputOutput::Key(uinput::event::keyboard::Key::J)),
        0x4B => Some(UInputOutput::Key(uinput::event::keyboard::Key::K)),
        0x4C => Some(UInputOutput::Key(uinput::event::keyboard::Key::L)),
        0x4D => Some(UInputOutput::Key(uinput::event::keyboard::Key::M)),
        0x4E => Some(UInputOutput::Key(uinput::event::keyboard::Key::N)),
        0x4F => Some(UInputOutput::Key(uinput::event::keyboard::Key::O)),
        0x50 => Some(UInputOutput::Key(uinput::event::keyboard::Key::P)),
        0x51 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Q)),
        0x52 => Some(UInputOutput::Key(uinput::event::keyboard::Key::R)),
        0x53 => Some(UInputOutput::Key(uinput::event::keyboard::Key::S)),
        0x54 => Some(UInputOutput::Key(uinput::event::keyboard::Key::T)),
        0x55 => Some(UInputOutput::Key(uinput::event::keyboard::Key::U)),
        0x56 => Some(UInputOutput::Key(uinput::event::keyboard::Key::V)),
        0x57 => Some(UInputOutput::Key(uinput::event::keyboard::Key::W)),
        0x58 => Some(UInputOutput::Key(uinput::event::keyboard::Key::X)),
        0x59 => Some(UInputOutput::Key(uinput::event::keyboard::Key::Y)),
        0x5A => Some(UInputOutput::Key(uinput::event::keyboard::Key::Z)),
        0x5B => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftMeta)),
        0x5C => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightMeta)),
        0x60 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_0)),
        0x61 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_1)),
        0x62 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_2)),
        0x63 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_3)),
        0x64 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_4)),
        0x65 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_5)),
        0x66 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_6)),
        0x67 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_7)),
        0x68 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_8)),
        0x69 => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::_9)),
        0x6A => Some(UInputOutput::KeyPad(
            uinput::event::keyboard::KeyPad::Asterisk,
        )),
        0x6B => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Plus)),
        0x6C => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Comma)),
        0x6D => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Minus)),
        0x6E => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Dot)),
        0x6F => Some(UInputOutput::KeyPad(uinput::event::keyboard::KeyPad::Slash)),
        0x70 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F1)),
        0x71 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F2)),
        0x72 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F3)),
        0x73 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F4)),
        0x74 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F5)),
        0x75 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F6)),
        0x76 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F7)),
        0x77 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F8)),
        0x78 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F9)),
        0x79 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F10)),
        0x7A => Some(UInputOutput::Key(uinput::event::keyboard::Key::F11)),
        0x7B => Some(UInputOutput::Key(uinput::event::keyboard::Key::F12)),
        0x7C => Some(UInputOutput::Key(uinput::event::keyboard::Key::F13)),
        0x7D => Some(UInputOutput::Key(uinput::event::keyboard::Key::F14)),
        0x7E => Some(UInputOutput::Key(uinput::event::keyboard::Key::F15)),
        0x7F => Some(UInputOutput::Key(uinput::event::keyboard::Key::F16)),
        0x80 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F17)),
        0x81 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F18)),
        0x82 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F19)),
        0x83 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F20)),
        0x84 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F21)),
        0x85 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F22)),
        0x86 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F23)),
        0x87 => Some(UInputOutput::Key(uinput::event::keyboard::Key::F24)),
        0x90 => Some(UInputOutput::Key(uinput::event::keyboard::Key::NumLock)),
        0x91 => Some(UInputOutput::Key(uinput::event::keyboard::Key::ScrollLock)),
        0xA0 => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftShift)),
        0xA1 => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightShift)),
        0xA2 => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftControl)),
        0xA3 => Some(UInputOutput::Key(
            uinput::event::keyboard::Key::RightControl,
        )),
        0xA4 => Some(UInputOutput::Key(uinput::event::keyboard::Key::LeftAlt)),
        0xA5 => Some(UInputOutput::Key(uinput::event::keyboard::Key::RightAlt)),
        0xBA => Some(UInputOutput::Key(uinput::event::keyboard::Key::SemiColon)),
        _ => None,
    }
}

/**
 * bus 0x3 vendor 0x45f product 0x2ea version 0x301
 * A button -> 304 (BTN_SOUTH, type 1 EV_KEY)
 * B button -> 305 (BTN_EAST, type 1 EV_KEY)
 * Y button -> 306 (BTN_WEST, type 1 EV_KEY)
 * X button -> 307 (BTN_NORTH, type 1 EV_KEY)
 * LB button -> 310 (BTN_TL, type 1 EV_KEY)
 * RB button -> 311 (BTN_TR, type 1 EV_KEY)
 * Left Stick button -> 317 (BTN_THUMBL, type 1 EV_KEY)
 * Right Stick button -> 318 (BTN_THUMBR, type 1 EV_KEY)
 * Select button -> 314 (BTN_SELECT, type 1 EV_KEY)
 * Start button -> 315 (BTN_START, type 1 EV_KEY)
 * Mode button -> 316 (BTN_MODE, type 1 EV_KEY)
 * DPAD Down -> 17, value 1 (ABS_HAT0Y, type 3 EV_ABS)
 * DPAD Up -> 17, value -1 (ABS_HAT0Y, type 3 EV_ABS)
 * DPAD Left -> 16, value -1 (ABS_HAT0X, type 3 EV_ABS)
 * DPAD Right -> 16, value 1 (ABS_HAT0X, type 3 EV_ABS)
 * Left x axis -> 0 (ABS_X, type 3 EV_ABS) [-32768, 32767]
 * Left y axis -> 1 (ABS_Y, type 3 EV_ABS) [-32768, 32767]
 * Right x axis -> 3 (ABS_RX, type 3 EV_ABS) [-32768, 32767]
 * Right y axis -> 4 (ABS_RY, type 3 EV_ABS) [-32768, 32767]
 * Left trigger -> 6 (ABS_Z, type 3 EV_ABS) [0, 1023]
 * Right trigger -> 5 (ABS_RZ, type 3 EV_ABS) [0, 1023]
 */

fn face_button_to_uinput_enum(code: u32) -> Option<UInputOutput> {
    match code {
        0x0010 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::Start,
        )),
        0x0020 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::Select,
        )),
        0x0040 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::ThumbL,
        )),
        0x0080 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::ThumbR,
        )),
        0x0100 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::TL,
        )),
        0x0200 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::TR,
        )),
        0x0400 => Some(UInputOutput::GamePad(
            uinput::event::controller::GamePad::Mode,
        )),
        0x1000 => Some(UInputOutput::GamePad(uinput::event::controller::GamePad::A)),
        0x2000 => Some(UInputOutput::GamePad(uinput::event::controller::GamePad::B)),
        0x4000 => Some(UInputOutput::GamePad(uinput::event::controller::GamePad::X)),
        0x8000 => Some(UInputOutput::GamePad(uinput::event::controller::GamePad::Y)),
        _ => None,
    }
}

fn dpad_button_to_abs_key(code: u32) -> Option<uinput::event::absolute::Hat> {
    match code {
        0x0001 => Some(uinput::event::absolute::Hat::Y0),
        0x0002 => Some(uinput::event::absolute::Hat::Y0),
        0x0004 => Some(uinput::event::absolute::Hat::X0),
        0x0008 => Some(uinput::event::absolute::Hat::X0),
        _ => None,
    }
}

fn dpad_button_to_abs_value(code: u32) -> Option<i32> {
    match code {
        0x0001 => Some(-1),
        0x0002 => Some(1),
        0x0004 => Some(-1),
        0x0008 => Some(1),
        _ => None,
    }
}

fn dpad_button_opposite_key(code: u32) -> Option<u32> {
    match code {
        0x0001 => Some(0x0002),
        0x0002 => Some(0x0001),
        0x0004 => Some(0x0008),
        0x0008 => Some(0x0004),
        _ => None,
    }
}

fn position_to_uinput_enum(result_type: &str) -> Option<uinput::event::absolute::Position> {
    match result_type {
        "thumb_lx" => Some(uinput::event::absolute::Position::X),
        "thumb_ly" => Some(uinput::event::absolute::Position::Y),
        "thumb_rx" => Some(uinput::event::absolute::Position::RX),
        "thumb_ry" => Some(uinput::event::absolute::Position::RY),
        "trigger_l" => Some(uinput::event::absolute::Position::Z),
        "trigger_r" => Some(uinput::event::absolute::Position::RZ),
        _ => None,
    }
}

fn check_focus(conn: &impl Connection, atoms: &Atoms, root_window: Window) -> String {
    let focus = conn
        .get_property(
            false,
            root_window,
            atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW,
            0,
            1,
        )
        .unwrap()
        .reply()
        .unwrap()
        .value32()
        .ok_or("_NET_ACTIVE_WINDOW has incorrect format")
        .unwrap()
        .next()
        .ok_or("_NET_ACTIVE_WINDOW is empty")
        .unwrap();

    let wm_class = WmClass::get(conn, focus).unwrap();
    let name = conn
        .get_property(false, focus, atoms.WM_NAME, atoms.STRING, 0, 0x1000)
        .unwrap();

    let name: String = String::from_utf8(name.reply().unwrap_or_default().value).unwrap();
    let wm_class = wm_class.reply().unwrap_or_default();
    if wm_class.is_some() {
        let wm_class = wm_class.unwrap();
        let class = std::str::from_utf8(wm_class.class()).unwrap();
        return class.to_string();
    } else {
        return name;
    }
}
