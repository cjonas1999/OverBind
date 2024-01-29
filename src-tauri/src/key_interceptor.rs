use once_cell::sync::Lazy;
use serde::Deserialize;
use std::cmp;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use vigem_client::Client;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{KBDLLHOOKSTRUCT_FLAGS, LLKHF_INJECTED};

use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    },
};

use crate::get_config_path;

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

struct SharedState {
    target: Option<vigem_client::Xbox360Wired<Client>>,
    hook_handle: Option<HHOOK>,
}

static SHARED_STATE: Lazy<Arc<RwLock<SharedState>>> = Lazy::new(|| {
    Arc::new(RwLock::new(SharedState {
        target: None,
        hook_handle: None,
    }))
});

pub(crate) struct KeyInterceptor {
    pub should_run: Arc<AtomicBool>,
}

impl KeyInterceptor {
    pub fn new() -> Self {
        Self {
            should_run: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn initialize(&mut self) -> Result<(), String> {
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

        Ok(())
    }

    pub fn start(&mut self) -> Result<(), String> {
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

        for item in &data {
            let keycode =
                u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");

            if item.result_type != "socd" {
                let key_state = key_states.entry(keycode).or_insert_with(|| KeyState {
                    is_pressed: false,
                    result_type: item.result_type.clone(),
                    result_value: item.result_value,
                });
                println!(
                    "Keycode: {:?}, ResultType: {:?}, ResultValue {:?}",
                    keycode, key_state.result_type, key_state.result_value
                );
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

                println!(
                    "Keycode: {:?}, OppositeKeycode: {:?}, OppositeKeyMapping: {:?}",
                    keycode,
                    opposite_key_state.opposite_key_value,
                    opposite_key_state.opposite_key_mapping
                );
            }
        }

        *KEY_STATES.write().unwrap() = key_states;
        *OPPOSITE_KEY_STATES.write().unwrap() = opposite_key_states;

        self.should_run.store(true, Ordering::SeqCst);

        // Set the hook and store the handle
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

        Ok(())
    }

    pub fn stop(&self) {
        let mut shared_state = SHARED_STATE.write().unwrap();
        if let Some(hook_handle) = shared_state.hook_handle.take() {
            unsafe {
                let _ = UnhookWindowsHookEx(hook_handle);
            }
        }
    }

    pub fn is_running(&self) -> bool {
        let shared_state = SHARED_STATE.read().unwrap();
        shared_state.hook_handle.is_some()
    }
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
    {
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
    {
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

                if opposite_key_state.opposite_key_type.clone() != String::from("face_button") {
                    let scan_code = MapVirtualKeyW(
                        cloned_key_state
                            .opposite_key_mapping
                            .unwrap_or(cloned_key_state.opposite_key_value as u16)
                            as u32,
                        MAPVK_VK_TO_VSC_EX,
                    ) as u16;
                    let ki = KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan_code,
                        dwFlags: KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE,
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

                if opposite_key_state.opposite_key_type.clone() != String::from("face_button") {
                    let scan_code = MapVirtualKeyW(
                        cloned_key_state
                            .opposite_key_mapping
                            .unwrap_or(cloned_key_state.opposite_key_value as u16)
                            as u32,
                        MAPVK_VK_TO_VSC_EX,
                    ) as u16;
                    let ki = KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan_code,
                        dwFlags: KEYEVENTF_SCANCODE,
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
                "left_trigger" => {
                    left_trigger = cmp::max(left_trigger, key_state.result_value as u8)
                }
                "right_trigger" => {
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

    return CallNextHookEx(None, n_code, w_param, l_param);
}
