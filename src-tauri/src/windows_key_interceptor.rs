#![cfg(target_os = "windows")]

use crate::key_interceptor::KeyInterceptorTrait;
use crate::text_masher::{
    text_masher, IS_MASHER_ACTIVE, MAX_MASHING_KEY_COUNT, SHOULD_TERMINATE_MASHER,
};
use crate::{get_config_path, Settings};
use log::{debug, error, info};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::{cmp, thread};
use vigem_client::Client;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, WriteFile, FILE_GENERIC_WRITE, FILE_SHARE_READ, OPEN_EXISTING,
    SECURITY_ANONYMOUS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, EVENT_OBJECT_FOCUS, KBDLLHOOKSTRUCT_FLAGS,
    LLKHF_INJECTED,
};
use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    },
};

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
    opposite_key_value: u32,
    opposite_key_mapping: Option<u16>,
}

static KEY_STATES: Lazy<Arc<RwLock<HashMap<u32, KeyState>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static OPPOSITE_KEY_STATES: Lazy<Arc<RwLock<HashMap<u32, OppositeKey>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static MASHING_KEYS: Lazy<Arc<RwLock<Vec<u32>>>> = Lazy::new(|| Arc::new(RwLock::new(Vec::new())));

struct SharedState {
    target: Option<vigem_client::Xbox360Wired<Client>>,
    hook_handle: Option<HHOOK>,
    window_hook_handle: Option<HWINEVENTHOOK>,
    allowed_programs: Option<Vec<String>>,
    block_kb_on_controller: bool,
}

static SHARED_STATE: Lazy<Arc<RwLock<SharedState>>> = Lazy::new(|| {
    Arc::new(RwLock::new(SharedState {
        target: None,
        hook_handle: None,
        window_hook_handle: None,
        allowed_programs: None,
        block_kb_on_controller: false,
    }))
});

pub(crate) struct WindowsKeyInterceptor {
    pub should_run: Arc<AtomicBool>,
}

impl KeyInterceptorTrait for WindowsKeyInterceptor {
    fn new() -> Self {
        Self {
            // TODO: Actually use this or remove it
            should_run: Arc::new(AtomicBool::new(false)),
        }
    }

    fn initialize(&mut self, settings: &Settings) -> Result<(), String> {
        println!("Initializing virtual controller");
        // Connect to the ViGEmBus driver
        let client = vigem_client::Client::connect().map_err(|e| e.to_string())?;
        // Create the virtual controller target
        let id = vigem_client::TargetId::XBOX360_WIRED;
        let mut target = vigem_client::Xbox360Wired::new(client, id);

        // Plugin the virtual controller
        target.plugin().map_err(|e| e.to_string())?;
        // Wait for the virtual controller to be ready to accept updates
        target.wait_ready().map_err(|e| e.to_string())?;

        let mut shared_state = SHARED_STATE.write().unwrap();
        shared_state.target = Some(target);
        if !settings.allowed_programs.is_empty() {
            info!("Allowed programs: {:?}", settings.allowed_programs);
            shared_state.allowed_programs = Some(settings.allowed_programs.clone());
        }
        shared_state.block_kb_on_controller = settings.block_kb_on_controller;

        Ok(())
    }

    fn start(&mut self, _: &tauri::AppHandle) -> Result<(), String> {
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
        let mut mashing_key_list = Vec::new();

        for item in &data {
            let keycode =
                u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");

            if item.result_type != "socd" {
                let key_state = key_states.entry(keycode).or_insert_with(|| KeyState {
                    is_pressed: false,
                    result_type: item.result_type.clone(),
                    result_value: item.result_value,
                });
                debug!(
                    "Keycode: {:?}, ResultType: {:?}, ResultValue {:?}",
                    keycode, key_state.result_type, key_state.result_value
                );
            }

            if item.result_type == "mash_trigger" {
                mashing_key_list.push(keycode);
            }
        }

        for item in &data {
            let keycode = u32::from_str_radix(&item.keycode, 16);

            if item.result_type == "socd" {
                let opposite_keycode = item.result_value as u32;

                // Check if key_state has a value for the opposite keycode and if so then use that value instead
                let key_state = key_states.get(&keycode.clone().unwrap());
                let mut key_type = String::from("keyboard");
                let mut key_mapping = None;

                if key_state.is_some()
                    && (key_state.unwrap().result_type == "keyboard"
                        || key_state.unwrap().result_type == "face_button")
                {
                    key_type = key_state.unwrap().result_type.clone();
                    key_mapping = Some(key_state.unwrap().result_value as u16);
                }

                let opposite_key_state = opposite_key_states
                    .entry(keycode.clone().unwrap())
                    .or_insert_with(|| OppositeKey {
                        is_pressed: false,
                        is_virtual_pressed: false,
                        opposite_key_value: opposite_keycode,
                        opposite_key_type: key_type,
                        opposite_key_mapping: key_mapping,
                    });

                debug!(
                    "Keycode: {:?}, OppositeKeycode: {:?}, OppositeKeyMapping: {:?}",
                    keycode,
                    opposite_key_state.opposite_key_value,
                    opposite_key_state.opposite_key_mapping
                );
            }
        }

        *KEY_STATES.write().unwrap() = key_states;
        *OPPOSITE_KEY_STATES.write().unwrap() = opposite_key_states;
        *MASHING_KEYS.write().unwrap() = mashing_key_list;

        self.should_run.store(true, Ordering::SeqCst);

        if MASHING_KEYS.read().unwrap().len() == MAX_MASHING_KEY_COUNT as usize {
            info!("SPAWNING MASHER THREAD");
            thread::spawn(|| {
                text_masher(
                    |key_to_press| {
                        if key_to_press > MAX_MASHING_KEY_COUNT {
                            for keycode in MASHING_KEYS.read().unwrap().iter() {
                                let _ = send_key_press(*keycode, false).map_err(|e| {
                                    error!("Error releasing key {:?}", e);
                                });
                            }
                        } else {
                            for (i, keycode) in MASHING_KEYS.read().unwrap().iter().enumerate() {
                                if (i as u8) == key_to_press {
                                    let _ = send_key_press(*keycode, true).map_err(|e| {
                                        error!("Error pressing key {:?}", e);
                                    });

                                    let _ = send_key_press(*keycode, false).map_err(|e| {
                                        error!("Error releasing key {:?}", e);
                                    });
                                }
                            }
                        }
                    },
                    toggle_masher_overlay,
                );
            });
        } else {
            info!("Not spawning masher thread, wrong number of mashing keys found.");
        }

        unsafe extern "system" fn win_event_proc(
            _hwineventhook: HWINEVENTHOOK,
            _event: u32,
            _hwnd: HWND,
            _idobject: i32,
            _idchild: i32,
            _ideventthread: u32,
            _dwmseventtime: u32,
        ) -> () {
            unsafe {
                let mut process_id = 0;
                let foreground_window = GetForegroundWindow();
                let _ = GetWindowThreadProcessId(foreground_window, Some(&mut process_id));
                let handle_result = OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    process_id,
                );
                let handle = match handle_result {
                    Ok(handle) => handle,
                    Err(err) => {
                        error!("Error opening process: {:?}", err);
                        return;
                    }
                };

                let mut buffer = [0u16; 1024];
                let mut size = buffer.len() as u32;
                let _ = QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_FORMAT(0),
                    PWSTR(buffer.as_mut_ptr()),
                    &mut size,
                );
                let full_process_name = String::from_utf16_lossy(&buffer[..size as usize]);
                let process_name = full_process_name.split('\\').last().unwrap();
                let _ = CloseHandle(handle);

                debug!("Active process: {:?}", process_name);

                let allowed_programs = SHARED_STATE.read().unwrap().allowed_programs.clone();
                for program in allowed_programs.unwrap() {
                    if process_name.contains(&program) {
                        if SHARED_STATE.read().unwrap().hook_handle == None {
                            info!("{:?} is allowed, activating hook", process_name);
                            let hook = SetWindowsHookExW(
                                WH_KEYBOARD_LL,
                                Some(low_level_keyboard_proc_callback),
                                HINSTANCE::default(),
                                0,
                            )
                            .map_err(|e| e.to_string());
                            let mut shared_state = SHARED_STATE.write().unwrap();
                            shared_state.hook_handle = Some(hook.unwrap());
                        }
                        return;
                    }
                }
            }
            {
                let mut shared_state = SHARED_STATE.write().unwrap();
                if let Some(hook_handle) = shared_state.hook_handle.take() {
                    info!("Deactivating hook");
                    unsafe {
                        let _ = UnhookWindowsHookEx(hook_handle);
                    }
                    shared_state.hook_handle = None;

                    {
                        let mut key_states = KEY_STATES.write().unwrap();

                        for (_, key_state) in key_states.iter_mut() {
                            key_state.is_pressed = false;
                        }
                    }

                    {
                        let mut opposite_key_states = OPPOSITE_KEY_STATES.write().unwrap();

                        for (_, key_state) in opposite_key_states.iter_mut() {
                            key_state.is_pressed = false;
                        }
                    }

                    let gamepad = vigem_client::XGamepad {
                        buttons: vigem_client::XButtons(0),
                        left_trigger: 0,
                        right_trigger: 0,
                        thumb_lx: 0,
                        thumb_ly: 0,
                        thumb_rx: 0,
                        thumb_ry: 0,
                    };

                    let mut temp_target = shared_state.target.take();
                    if let Some(ref mut target) = &mut temp_target {
                        let _ = target.update(&gamepad);

                        shared_state.target = temp_target;
                    }
                }
            }
        }

        let allowed_programs = SHARED_STATE.read().unwrap().allowed_programs.clone();
        if allowed_programs.is_some() {
            info!("Starting window hook");
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_OBJECT_FOCUS,
                    EVENT_OBJECT_FOCUS,
                    HINSTANCE::default(),
                    Some(win_event_proc),
                    0,
                    0,
                    0,
                )
            };
            let mut shared_state = SHARED_STATE.write().unwrap();
            shared_state.window_hook_handle = Some(hook);
        } else {
            info!("Starting key interception");
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc_callback),
                    HINSTANCE::default(),
                    0,
                )
            }
            .map_err(|e| e.to_string())?;
            let mut shared_state = SHARED_STATE.write().unwrap();
            shared_state.hook_handle = Some(hook);
        }

        Ok(())
    }

    fn stop(&self, _: &tauri::AppHandle) {
        let mut shared_state = SHARED_STATE.write().unwrap();
        if let Some(hook_handle) = shared_state.hook_handle.take() {
            info!("Stopping hook");
            unsafe {
                let _ = UnhookWindowsHookEx(hook_handle);
            }
            shared_state.hook_handle = None;
        }
        if let Some(window_hook_handle) = shared_state.window_hook_handle.take() {
            info!("Stopping window hook");
            unsafe {
                let _ = UnhookWinEvent(window_hook_handle);
            }
            shared_state.window_hook_handle = None;
        }

        SHOULD_TERMINATE_MASHER.store(true, Ordering::SeqCst);
    }

    fn is_running(&self) -> bool {
        let shared_state = SHARED_STATE.read().unwrap();
        shared_state.hook_handle.is_some() || shared_state.window_hook_handle.is_some()
    }
}

fn toggle_masher_overlay(active: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut command = None;
    if active {
        debug!("Showing masher overlay");
        command = Some("masher_active");
    } else {
        debug!("Hiding masher overlay");
        command = Some("masher_inactive");
    }
    let pipe_name = r"\\.\pipe\masher_overlay_v2.0.2-beta";
    let name_w: Vec<u16> = OsStr::new(pipe_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(name_w.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            SECURITY_ANONYMOUS,
            None,
        )?;

        let mut written = 0u32;
        WriteFile(
            handle,
            Some(command.unwrap().as_bytes()),
            Some(&mut written as *mut u32),
            None,
        )?;

        let _ = FlushFileBuffers(handle);
        let _ = CloseHandle(handle);
    }

    Ok(())
}

fn send_key_press(keycode: u32, is_keydown: bool) -> Result<(), windows::core::Error> {
    unsafe {
        let extended_flag = if is_extended_key(keycode) {
            KEYEVENTF_EXTENDEDKEY
        } else {
            KEYBD_EVENT_FLAGS(0)
        };
        let keyup_flag = if is_keydown {
            KEYBD_EVENT_FLAGS(0)
        } else {
            KEYEVENTF_KEYUP
        };

        let scan_code = MapVirtualKeyW(keycode, MAPVK_VK_TO_VSC_EX) as u16;

        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan_code,
                    dwFlags: KEYEVENTF_SCANCODE | keyup_flag | extended_flag,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };

        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
    Ok(())
}

fn is_extended_key(virtual_keycode: u32) -> bool {
    let extended_keys: [u32; 14] = [
        0x21, //page up
        0x22, //page down
        0x23, //end
        0x24, //home
        0x25, //left arrow
        0x26, //up arrow
        0x27, //right arrow
        0x28, //down arrow
        0x2C, //print screen
        0x2D, //insert
        0x2E, //delete
        0x90, //numlock
        0xA3, //right CTRL
        0xA5, //right ALT
    ];
    extended_keys.contains(&virtual_keycode)
}

// Used for when multiple stick rebinds are set, first prioritizes right/up(positive) over left/down(negative), then larger values within those bands
// neutral(zero) is the absolute lowest priority
fn analog_priority_transform(n: i32) -> i32 {
    if n < 0 {
        return -1 * n;
    }
    if n > 0 {
        return n + i16::MAX as i32;
    }

    0
}

fn find_higher_priority(num1: i16, num2: i16) -> i16 {
    if num1 == 0 {
        return num2;
    }
    if num2 == 0 {
        return num1;
    }

    if analog_priority_transform(num1 as i32) >= analog_priority_transform(num2 as i32) {
        return num1;
    } else {
        return num2;
    }
}

unsafe extern "system" fn low_level_keyboard_proc_callback(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let kbd_struct = l_param.0 as *const KBDLLHOOKSTRUCT;
    if n_code != HC_ACTION as i32
        || (*kbd_struct).flags & LLKHF_INJECTED != KBDLLHOOKSTRUCT_FLAGS(0)
    {
        return CallNextHookEx(HHOOK::default(), n_code, w_param, l_param);
    }
    let key = (*kbd_struct).vkCode;
    let key_is_down = match w_param.0 as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return CallNextHookEx(None, n_code, w_param, l_param),
    };

    let mut disable_keyboard = false;
    {
        let shared_state = SHARED_STATE.write().unwrap();
        disable_keyboard = shared_state.block_kb_on_controller
    }

    // Update Key State
    let mut key_event_flag = None;
    {
        let mut key_states = KEY_STATES.write().unwrap();
        match key_states.get_mut(&key) {
            Some(state) => state.is_pressed = key_is_down,
            _ => (),
        }
        key_event_flag = Some(if key_is_down {
            KEYBD_EVENT_FLAGS(0)
        } else {
            KEYEVENTF_KEYUP
        });
    }

    // Keyboard Rebinds
    'rebind: {
        if disable_keyboard {
            break 'rebind;
        }
        let key_states = KEY_STATES.read().unwrap();
        if let Some(flag) = key_event_flag {
            if let Some(key_state) = key_states.get(&key) {
                if key_state.result_type == "keyboard" {
                    let vk_code = key_state.result_value as u16;

                    let ki = KEYBDINPUT {
                        wVk: VIRTUAL_KEY(vk_code),
                        wScan: 0,
                        dwFlags: flag,
                        time: 0,
                        dwExtraInfo: 0,
                    };

                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 { ki },
                    };

                    unsafe {
                        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                    }

                    return LRESULT(1);
                }
            }
        }
    }

    // SOCD
    'socd: {
        if disable_keyboard {
            break 'socd;
        }

        let mut opposite_key_states = OPPOSITE_KEY_STATES.write().unwrap();

        let cloned_key_state;
        if opposite_key_states.contains_key(&key) {
            {
                let key_state = opposite_key_states.get_mut(&key).unwrap();
                key_state.is_pressed = key_is_down;
                key_state.is_virtual_pressed = key_is_down;

                cloned_key_state = key_state.clone();
            }

            let opposite_key_state = opposite_key_states
                .get_mut(&cloned_key_state.opposite_key_value)
                .unwrap();

            if key_is_down && opposite_key_state.is_pressed && opposite_key_state.is_virtual_pressed
            {
                opposite_key_state.is_virtual_pressed = false;

                // if opposite_key_state.opposite_key_type.clone() != String::from("face_button") {
                {
                    let extended_flag;
                    let scan_code = {
                        let key_value = if cloned_key_state.opposite_key_type.clone()
                            == String::from("face_button")
                            || cloned_key_state.opposite_key_mapping.is_none()
                        {
                            cloned_key_state.opposite_key_value as u32
                        } else {
                            cloned_key_state.opposite_key_mapping.unwrap() as u32
                        };

                        extended_flag = if is_extended_key(key_value) {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        };

                        let code = MapVirtualKeyW(key_value, MAPVK_VK_TO_VSC_EX);

                        code as u16
                    };

                    let ki = KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan_code,
                        dwFlags: KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE | extended_flag,
                        time: 0,
                        dwExtraInfo: 0,
                    };

                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 { ki },
                    };

                    unsafe {
                        let inputs = &[input];
                        let inputs_size = std::mem::size_of::<INPUT>() as i32;
                        SendInput(inputs, inputs_size);
                    }
                }
            } else if !key_is_down && opposite_key_state.is_pressed {
                opposite_key_state.is_virtual_pressed = true;

                // if opposite_key_state.opposite_key_type.clone() != String::from("face_button") {
                {
                    let extended_flag;
                    let scan_code = {
                        let key_value = if cloned_key_state.opposite_key_type.clone()
                            == String::from("face_button")
                            || cloned_key_state.opposite_key_mapping.is_none()
                        {
                            cloned_key_state.opposite_key_value as u32
                        } else {
                            cloned_key_state.opposite_key_mapping.unwrap() as u32
                        };

                        extended_flag = if is_extended_key(key_value) {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        };

                        let code = MapVirtualKeyW(key_value, MAPVK_VK_TO_VSC_EX);

                        code as u16
                    };

                    let ki = KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan_code,
                        dwFlags: KEYEVENTF_SCANCODE | extended_flag,
                        time: 0,
                        dwExtraInfo: 0,
                    };

                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 { ki },
                    };

                    unsafe {
                        let inputs = &[input];
                        let inputs_size = std::mem::size_of::<INPUT>() as i32;
                        SendInput(inputs, inputs_size);
                    }
                }
            }
        }
    }

    // Mash Trigger State Update
    {
        let key_states = KEY_STATES.read().unwrap();
        if matches!(key_states.get(&key), Some(state) if state.result_type == "mash_trigger") {
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

    // Controller Rebinds
    let mut face_buttons: u16 = 0;
    let mut left_trigger: u8 = 0;
    let mut right_trigger: u8 = 0;
    let mut thumb_lx: i16 = 0;
    let mut thumb_ly: i16 = 0;
    let mut thumb_rx: i16 = 0;
    let mut thumb_ry: i16 = 0;

    {
        let key_states = KEY_STATES.read().unwrap();
        for (_, key_state) in key_states.iter().filter(|&(_, ks)| ks.is_pressed) {
            match key_state.result_type.as_str() {
                "face_button" => face_buttons = face_buttons | key_state.result_value as u16,
                "trigger_l" => left_trigger = cmp::max(left_trigger, key_state.result_value as u8),
                "trigger_r" => {
                    right_trigger = cmp::max(right_trigger, key_state.result_value as u8)
                }
                "thumb_lx" => {
                    thumb_lx = find_higher_priority(thumb_lx, key_state.result_value as i16)
                }
                "thumb_ly" => {
                    thumb_ly = find_higher_priority(thumb_ly, key_state.result_value as i16)
                }
                "thumb_rx" => {
                    thumb_rx = find_higher_priority(thumb_rx, key_state.result_value as i16)
                }
                "thumb_ry" => {
                    thumb_ry = find_higher_priority(thumb_ry, key_state.result_value as i16)
                }
                _ => (),
            }
        }
    }

    {
        let opposite_key_states = OPPOSITE_KEY_STATES.read().unwrap();
        for (_, opposite_key_state) in opposite_key_states
            .iter()
            .filter(|&(_, ks)| ks.opposite_key_type == String::from("face_button"))
        {
            if let Some(opposite_key_mapping) = opposite_key_state.opposite_key_mapping {
                let mask = opposite_key_mapping as u16;

                if opposite_key_state.is_virtual_pressed {
                    // If it's virtually pressed, set the specific bit
                    face_buttons |= mask;
                } else {
                    // If it's virtually unpressed, clear the specific bit
                    let clear_mask = !mask;
                    face_buttons &= clear_mask;
                }
            }
        }
    }

    let gamepad = vigem_client::XGamepad {
        buttons: vigem_client::XButtons(face_buttons),
        left_trigger: left_trigger,
        right_trigger: right_trigger,
        thumb_lx: thumb_lx,
        thumb_ly: thumb_ly,
        thumb_rx: thumb_rx,
        thumb_ry: thumb_ry,
    };

    {
        let mut shared_state = SHARED_STATE.write().unwrap();
        let mut temp_target = shared_state.target.take();
        if let Some(ref mut target) = &mut temp_target {
            let _ = target.update(&gamepad);

            shared_state.target = temp_target;
        }
    }

    if disable_keyboard {
        return LRESULT(1);
    }

    return CallNextHookEx(None, n_code, w_param, l_param);
}
