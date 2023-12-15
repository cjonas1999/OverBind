use once_cell::sync::Lazy;
use serde::Deserialize;
use std::cmp;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vigem_client::Client;

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
    results: Vec<KeyResult>
}

struct KeyResult {
    result_type: String,
    result_value: i32
}

static KEY_STATES: Lazy<Arc<Mutex<HashMap<u32, KeyState>>>> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

struct SharedState {
    target: Option<vigem_client::Xbox360Wired<Client>>,
    hook_handle: Option<HHOOK>,
}

static SHARED_STATE: Lazy<Arc<Mutex<SharedState>>> = Lazy::new(|| {
    Arc::new(Mutex::new(SharedState {
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


        let mut shared_state = SHARED_STATE.lock().unwrap();
        shared_state.target = Some(target);

        Ok(())
    }

    pub fn start(&mut self) -> Result<(), String> {
        // Read keybindings from file
        let path = Path::new("OverBind_conf.json");
        let mut file = File::open(&path).map_err(|e| e.to_string())?;
        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| e.to_string())?;
        
        let data: Vec<ConfigJsonData> = serde_json::from_str(&contents).map_err(|e| e.to_string())?;
        let mut key_states = HashMap::new();

        for item in data {
            let keycode = u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");
            let key_result = KeyResult {
                result_type: item.result_type,
                result_value: item.result_value
            };

            let key_state = key_states
            .entry(keycode)
            .or_insert_with(|| KeyState {
                is_pressed: false,
                results: Vec::new(),
            });

            key_state.results.push(key_result);
        }

        *KEY_STATES.lock().unwrap() = key_states;

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
        let mut shared_state = SHARED_STATE.lock().unwrap();
        shared_state.hook_handle = Some(hook);

        Ok(())
    }

    pub fn stop(&self) {
        let mut shared_state = SHARED_STATE.lock().unwrap();
        if let Some(hook_handle) = shared_state.hook_handle.take() {
            unsafe {
                let _ = UnhookWindowsHookEx(hook_handle);
            }
        }
    }

    pub fn is_running(&self) -> bool {
        let shared_state = SHARED_STATE.lock().unwrap();
        shared_state.hook_handle.is_some()
    }
}


// Used for when multiple stick rebinds are set, first prioritizes right/up(positive) over left/down(negative), then larger values within those bands
// neutral(zero) is the absolute lowest priority
fn analog_priority_transform(n: i32) -> i32 {
    if n < 0 {
        return -1 * n
    }
    if n > 0 {
        return n + i16::MAX as i32
    }

    0
}

fn find_higher_priority(num1: i16, num2: i16) -> i16 {
    if num1 == 0 {
        return num2
    }
    if num2 == 0{
        return num1
    }

    if analog_priority_transform(num1 as i32) >= analog_priority_transform(num2 as i32) {
        return num1
    }
    else {
        return num2
    }
}

unsafe extern "system" fn low_level_keyboard_proc_callback(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let mut key_states = KEY_STATES.lock().unwrap();
    let mut shared_state = SHARED_STATE.lock().unwrap();
    let mut temp_target = shared_state.target.take();
    let hook_handle = shared_state.hook_handle.unwrap();

    if n_code != HC_ACTION as i32 {
        return CallNextHookEx(HHOOK::default(), n_code, w_param, l_param);
    }

    let kbd_struct = l_param.0 as *const KBDLLHOOKSTRUCT;
    //println!("Key {:?} {:?}", w_param, (*kbd_struct).vkCode);

    match w_param.0 as u32 {
        WM_KEYUP | WM_SYSKEYUP => {
            match key_states.get_mut(&(*kbd_struct).vkCode) {
                Some(state) => {state.is_pressed = false},
                _ => ()
            }
        }

        WM_KEYDOWN | WM_SYSKEYDOWN => {
            match key_states.get_mut(&(*kbd_struct).vkCode) {
                Some(state) => {state.is_pressed = true},
                _ => ()
            }
        }

        _ => return CallNextHookEx(HHOOK::default(), n_code, w_param, l_param),
    }


    let mut face_buttons: u16 = 0;
    let mut left_trigger: u8 = 0;
    let mut right_trigger: u8 = 0;
    let mut thumb_lx: i16 = 0;
    let mut thumb_ly: i16 = 0;
    let mut thumb_rx: i16 = 0;
    let mut thumb_ry: i16 = 0;

    for (_, key_state) in key_states.iter().filter(|&(_, ks)| ks.is_pressed) {
        for result in key_state.results.iter() {
            match result.result_type.as_str() {
                "face_button" => { face_buttons = face_buttons | result.result_value as u16 },
                "left_trigger" => { left_trigger = cmp::max(left_trigger, result.result_value as u8) },
                "right_trigger" => { right_trigger = cmp::max(right_trigger, result.result_value as u8) },
                "thumb_lx" => { thumb_lx = find_higher_priority(thumb_lx, result.result_value as i16) },
                "thumb_ly" => { thumb_ly = find_higher_priority(thumb_ly, result.result_value as i16) },
                "thumb_rx" => { thumb_rx = find_higher_priority(thumb_rx, result.result_value as i16) },
                "thumb_ry" => { thumb_ry = find_higher_priority(thumb_ry, result.result_value as i16) },
                _ => ()
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
        thumb_ry: thumb_ry
    };

    if let Some(ref mut target) = &mut temp_target {
        let _ = target.update(&gamepad);

        shared_state.target = temp_target;
    }

    return CallNextHookEx(hook_handle, n_code, w_param, l_param);
}
