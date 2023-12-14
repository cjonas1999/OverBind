use once_cell::sync::Lazy;
use std::fs::File;
use std::io::{BufRead, BufReader};
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

static STICK_NEUTRAL: i16 = 0;
static STICK_LEFT: i16 = -29000;
static STICK_RIGHT: i16 = 29000;
static STICK_UP: i16 = 29000;
static STICK_DOWN: i16 = -29000;
static KEYBIND_LEFT_STICK_LEFT: usize = 0;
static KEYBIND_LEFT_STICK_RIGHT: usize = 1;
static KEYBIND_RIGHT_STICK_UP: usize = 2;

static KEYBINDS: Lazy<Arc<Mutex<Vec<u32>>>> = Lazy::new(|| Arc::new(Mutex::new(vec![])));
static mut KEY_HELD: [bool; 3] = [false; 3];

struct SharedState {
    target: Option<vigem_client::Xbox360Wired<Client>>,
    gamepad: Option<vigem_client::XGamepad>,
    hook_handle: Option<HHOOK>,
}

static SHARED_STATE: Lazy<Arc<Mutex<SharedState>>> = Lazy::new(|| {
    Arc::new(Mutex::new(SharedState {
        target: None,
        gamepad: None,
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

        // Initialize the gamepad state
        let gamepad = vigem_client::XGamepad {
            ..Default::default()
        };

        let mut shared_state = SHARED_STATE.lock().unwrap();
        shared_state.target = Some(target);
        shared_state.gamepad = Some(gamepad);

        Ok(())
    }

    pub fn start(&mut self) -> Result<(), String> {
        // Read keybindings from file
        let path = Path::new("OverBind_conf.txt");
        let file = File::open(&path).map_err(|e| e.to_string())?;
        let reader = BufReader::new(file);
        let mut keybinds = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?.trim().to_owned();

            if let Ok(value) = u32::from_str_radix(&line, 16) {
                keybinds.push(value);
            } else {
                return Err(format!("Failed to parse keybinding: {}", line));
            }
        }
        *KEYBINDS.lock().unwrap() = keybinds;

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

unsafe extern "system" fn low_level_keyboard_proc_callback(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let keybinds = KEYBINDS.lock().unwrap();
    let mut shared_state = SHARED_STATE.lock().unwrap();
    let mut temp_target = shared_state.target.take();
    let mut temp_gamepad = shared_state.gamepad.take();
    let hook_handle = shared_state.hook_handle.unwrap();

    if n_code != HC_ACTION as i32 {
        return CallNextHookEx(HHOOK::default(), n_code, w_param, l_param);
    }

    let kbd_struct = l_param.0 as *const KBDLLHOOKSTRUCT;
    println!("Key {:?} {:?}", w_param, (*kbd_struct).vkCode);

    match w_param.0 as u32 {
        WM_KEYUP | WM_SYSKEYUP => {
            for i in 0..3 {
                if (*kbd_struct).vkCode == keybinds[i] {
                    KEY_HELD[i] = false;
                }
            }
        }

        WM_KEYDOWN | WM_SYSKEYDOWN => {
            for i in 0..3 {
                if (*kbd_struct).vkCode == keybinds[i] {
                    KEY_HELD[i] = true;
                }
            }
        }

        _ => return CallNextHookEx(HHOOK::default(), n_code, w_param, l_param),
    }

    let mut left_stick_x: i16 = STICK_NEUTRAL;
    let mut left_stick_y: i16 = STICK_NEUTRAL;
    let mut right_stick_x: i16 = STICK_NEUTRAL;
    let mut right_stick_y: i16 = STICK_NEUTRAL;

    if KEY_HELD[KEYBIND_LEFT_STICK_RIGHT] {
        left_stick_x = STICK_RIGHT;
    } else if KEY_HELD[KEYBIND_LEFT_STICK_LEFT] {
        left_stick_x = STICK_LEFT;
    } else {
        left_stick_x = STICK_NEUTRAL;
    }

    if KEY_HELD[KEYBIND_RIGHT_STICK_UP] {
        right_stick_y = STICK_UP;
    } else {
        right_stick_y = STICK_NEUTRAL;
    }

    if let (Some(ref mut target), Some(ref mut gamepad)) = (&mut temp_target, &mut temp_gamepad) {
        gamepad.thumb_lx = left_stick_x;
        gamepad.thumb_ly = left_stick_y;
        gamepad.thumb_rx = right_stick_x;
        gamepad.thumb_ry = right_stick_y;
        let _ = target.update(&gamepad);

        shared_state.target = temp_target;
        shared_state.gamepad = temp_gamepad;
    }

    return CallNextHookEx(hook_handle, n_code, w_param, l_param);
}
