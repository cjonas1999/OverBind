#![cfg(target_os = "windows")]

use log::{debug, error, info};
use once_cell::sync::Lazy;
use sdl3::event::Event;
use sdl3::gamepad;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::{cmp, thread};
use vigem_client::{Client, XGamepad};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
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

use crate::key_interceptor::KeyInterceptorTrait;
use crate::text_masher::{text_masher, IS_MASHER_ACTIVE, MAX_MASHING_KEY_COUNT};
use crate::{get_config_path, Settings};

struct SharedState {
    gamepad_state: Option<vigem_client::XGamepad>,
    target: Option<vigem_client::Xbox360Wired<Client>>,
    allowed_programs: Option<Vec<String>>,
}

static SHARED_STATE: Lazy<Arc<RwLock<SharedState>>> = Lazy::new(|| {
    Arc::new(RwLock::new(SharedState {
        gamepad_state: None,
        target: None,
        allowed_programs: None,
    }))
});

static MASHING_BUTTONS: Lazy<Arc<RwLock<Vec<MashTrigger>>>> =
    Lazy::new(|| Arc::new(RwLock::new(Vec::new())));

enum MashTrigger {
    Button(u16),
    LeftTrigger,
    RightTrigger,
}

static BUTTON_MAPPINGS: [(sdl3::gamepad::Button, u16); 15] = [
    (sdl3::gamepad::Button::North, vigem_client::XButtons::Y),
    (sdl3::gamepad::Button::East, vigem_client::XButtons::B),
    (sdl3::gamepad::Button::South, vigem_client::XButtons::A),
    (sdl3::gamepad::Button::West, vigem_client::XButtons::X),
    (sdl3::gamepad::Button::Back, vigem_client::XButtons::BACK),
    (sdl3::gamepad::Button::Guide, vigem_client::XButtons::GUIDE),
    (sdl3::gamepad::Button::Start, vigem_client::XButtons::START),
    (
        sdl3::gamepad::Button::LeftStick,
        vigem_client::XButtons::LTHUMB,
    ),
    (
        sdl3::gamepad::Button::RightStick,
        vigem_client::XButtons::RTHUMB,
    ),
    (
        sdl3::gamepad::Button::LeftShoulder,
        vigem_client::XButtons::LB,
    ),
    (
        sdl3::gamepad::Button::RightShoulder,
        vigem_client::XButtons::RB,
    ),
    (sdl3::gamepad::Button::DPadUp, vigem_client::XButtons::UP),
    (
        sdl3::gamepad::Button::DPadDown,
        vigem_client::XButtons::DOWN,
    ),
    (
        sdl3::gamepad::Button::DPadLeft,
        vigem_client::XButtons::LEFT,
    ),
    (
        sdl3::gamepad::Button::DPadRight,
        vigem_client::XButtons::RIGHT,
    ),
];

pub(crate) struct WindowsGamepadInterceptor {
    pub should_run: Arc<AtomicBool>,
}

impl KeyInterceptorTrait for WindowsGamepadInterceptor {
    fn new() -> Self {
        Self {
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
        shared_state.gamepad_state = Some(vigem_client::XGamepad::default());
        if !settings.allowed_programs.is_empty() {
            info!("Allowed programs: {:?}", settings.allowed_programs);
            shared_state.allowed_programs = Some(settings.allowed_programs.clone());
        }

        Ok(())
    }

    fn start(&mut self, _: &tauri::AppHandle) -> Result<(), String> {
        let mut mashing_key_list = Vec::new();
        mashing_key_list.push(MashTrigger::Button(vigem_client::XButtons::Y as u16));
        mashing_key_list.push(MashTrigger::Button(vigem_client::XButtons::B as u16));
        mashing_key_list.push(MashTrigger::LeftTrigger);
        *MASHING_BUTTONS.write().unwrap() = mashing_key_list;

        // TODO: mashing thread
        if MASHING_BUTTONS.read().unwrap().len() == MAX_MASHING_KEY_COUNT as usize {
            info!("SPAWNING MASHER THREAD");
            thread::spawn(|| {
                text_masher(|key_to_press| {
                    {
                        let mut shared_state = SHARED_STATE.write().unwrap();

                        let mut temp_target = shared_state.target.take();
                        let mut gamepad_state =
                            shared_state.gamepad_state.take().unwrap_or_default();

                        let mut face_buttons: u16 = gamepad_state.buttons.raw;
                        let thumb_lx = gamepad_state.thumb_lx;
                        let thumb_ly = gamepad_state.thumb_ly;
                        let thumb_rx = gamepad_state.thumb_rx;
                        let thumb_ry = gamepad_state.thumb_ry;
                        let mut left_trigger = gamepad_state.left_trigger;
                        let mut right_trigger = gamepad_state.right_trigger;

                        if key_to_press > MAX_MASHING_KEY_COUNT {
                            // stop mashing
                            for input in MASHING_BUTTONS.read().unwrap().iter() {
                                match input {
                                    MashTrigger::Button(b) => face_buttons &= !b,
                                    MashTrigger::LeftTrigger => left_trigger = 0,
                                    MashTrigger::RightTrigger => right_trigger = 0,
                                }
                            }
                        } else {
                            for (i, input) in MASHING_BUTTONS.read().unwrap().iter().enumerate() {
                                info!("{}", key_to_press);
                                if (i as u8) == key_to_press {
                                    match input {
                                        MashTrigger::Button(b) => face_buttons |= b,
                                        MashTrigger::LeftTrigger => {
                                            left_trigger = {
                                                info!("LEFT TRIG PRESS!");
                                                u8::max_value()
                                            }
                                        }
                                        MashTrigger::RightTrigger => {
                                            right_trigger = u8::max_value()
                                        }
                                    }
                                } else if (i as u8) == (key_to_press + 1) % MAX_MASHING_KEY_COUNT {
                                    match input {
                                        MashTrigger::Button(b) => face_buttons &= !b,
                                        MashTrigger::LeftTrigger => left_trigger = 0,
                                        MashTrigger::RightTrigger => right_trigger = 0,
                                    }
                                }
                            }
                        }

                        // update gamepad state
                        gamepad_state = vigem_client::XGamepad {
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
                                let _ = target.update(&gamepad_state);

                                shared_state.target = temp_target;
                            }
                        }
                    }
                });
            });
        } else {
            info!("Not spawning masher thread, wrong number of mashing keys found.");
        }
        // Controller Input Polling
        thread::spawn(|| {
            sdl3::hint::set("SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
            let sdl_context = sdl3::init().unwrap();
            let gamepad_system = sdl_context.gamepad().unwrap();
            let gamepads = gamepad_system.gamepads().unwrap();

            let expected_id = "0300fa67c82d00000631000014017801";
            info!("expected_id: {}", expected_id);

            loop {
                // Identify gamepad with correct id
                let pad_opt = gamepads.iter().find(|&joystick_id| {
                    info!(
                        "guid_for_id: {:?} {:?}",
                        gamepad_system.guid_for_id(*joystick_id).string(),
                        gamepad_system.product_version_for_id(*joystick_id)
                    );
                    gamepad_system.guid_for_id(*joystick_id).string() == expected_id
                });
                if let Ok(pad) = gamepad_system.open(*pad_opt.unwrap()) {
                    let mut gamepad_state = vigem_client::XGamepad::default();

                    loop {
                        gamepad_system.update();

                        // Translate real gamepad inputs to virtual gamepad inputs
                        let mut face_buttons: u16 = 0;
                        let thumb_lx = pad.axis(sdl3::gamepad::Axis::LeftX);
                        let thumb_ly = pad.axis(sdl3::gamepad::Axis::LeftY);
                        let thumb_rx = pad.axis(sdl3::gamepad::Axis::RightX);
                        let thumb_ry = pad.axis(sdl3::gamepad::Axis::RightY);
                        let mut left_trigger =
                            (pad.axis(sdl3::gamepad::Axis::TriggerLeft) / 128) as u8;
                        let mut right_trigger =
                            (pad.axis(sdl3::gamepad::Axis::TriggerRight) / 128) as u8;

                        for (sdl_button, vigem_button) in BUTTON_MAPPINGS.iter() {
                            if pad.button(*sdl_button) {
                                face_buttons = face_buttons | *vigem_button as u16;
                            }
                        }

                        // Detect Mashing Buttons
                        let mut all_pressed = true;
                        if MASHING_BUTTONS.read().unwrap().len() != 0 {
                            for button in MASHING_BUTTONS.read().unwrap().iter() {
                                match button {
                                    MashTrigger::Button(b) => {
                                        all_pressed &= (face_buttons & *b) > 0;
                                    }
                                    MashTrigger::LeftTrigger => {
                                        all_pressed &= left_trigger > 0;
                                    }
                                    MashTrigger::RightTrigger => {
                                        all_pressed &= right_trigger > 0;
                                    }
                                }
                            }
                        } else {
                            //no mashing keys configured
                            all_pressed = false
                        }

                        if IS_MASHER_ACTIVE.load(Ordering::SeqCst) != all_pressed {
                            IS_MASHER_ACTIVE.store(all_pressed, Ordering::SeqCst);
                        }
                        if all_pressed {
                            info!("all_pressed: {:?}", all_pressed);
                            // revert mashing buttons to previous state so the mashing code has
                            // full control
                            for button in MASHING_BUTTONS.read().unwrap().iter() {
                                match button {
                                    MashTrigger::Button(b) => {
                                        let prev_button_state = gamepad_state.buttons.raw & *b;
                                        if prev_button_state > 0 {
                                            face_buttons &= *b;
                                        } else {
                                            face_buttons |= *b;
                                        }
                                    }
                                    MashTrigger::LeftTrigger => {
                                        left_trigger = gamepad_state.left_trigger;
                                    }
                                    MashTrigger::RightTrigger => {
                                        right_trigger = gamepad_state.right_trigger;
                                    }
                                }
                            }
                        }

                        // update gamepad state
                        gamepad_state = vigem_client::XGamepad {
                            buttons: vigem_client::XButtons(face_buttons),
                            left_trigger: left_trigger,
                            right_trigger: right_trigger,
                            thumb_lx: thumb_lx,
                            thumb_ly: thumb_ly.saturating_neg(),
                            thumb_rx: thumb_rx,
                            thumb_ry: thumb_ry.saturating_neg(),
                        };
                        {
                            let mut shared_state = SHARED_STATE.write().unwrap();
                            let mut temp_target = shared_state.target.take();
                            if let Some(ref mut target) = &mut temp_target {
                                let _ = target.update(&gamepad_state);

                                shared_state.target = temp_target;
                            }
                        }

                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                }

                thread::sleep(std::time::Duration::from_millis(1000));
            }
        });

        Ok(())
    }

    fn stop(&self, _: &tauri::AppHandle) {
        todo!()
    }

    fn is_running(&self) -> bool {
        true
    }
}
