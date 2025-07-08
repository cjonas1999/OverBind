#![cfg(target_os = "windows")]

use log::{debug, error, info};
use once_cell::sync::Lazy;
use sdl3::gamepad;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::{cmp, thread};
use vigem_client::Client;
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

enum SdlInput {
    Button(sdl3::gamepad::Button),
    Axis(sdl3::gamepad::Axis),
}

struct SdlToVigemButton {
    sdl: sdl3::gamepad::Button,
    vigem: u16,
}

static SDL_TO_VIGEM_BUTTON: [SdlToVigemButton; 15] = [
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::North,
        vigem: vigem_client::XButtons::Y,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::East,
        vigem: vigem_client::XButtons::B,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::South,
        vigem: vigem_client::XButtons::A,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::West,
        vigem: vigem_client::XButtons::X,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::Back,
        vigem: vigem_client::XButtons::BACK,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::Guide,
        vigem: vigem_client::XButtons::GUIDE,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::Start,
        vigem: vigem_client::XButtons::START,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::LeftStick,
        vigem: vigem_client::XButtons::LTHUMB,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::RightStick,
        vigem: vigem_client::XButtons::RTHUMB,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::LeftShoulder,
        vigem: vigem_client::XButtons::LB,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::RightShoulder,
        vigem: vigem_client::XButtons::RB,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::DPadUp,
        vigem: vigem_client::XButtons::UP,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::DPadDown,
        vigem: vigem_client::XButtons::DOWN,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::DPadLeft,
        vigem: vigem_client::XButtons::LEFT,
    },
    SdlToVigemButton {
        sdl: sdl3::gamepad::Button::DPadRight,
        vigem: vigem_client::XButtons::RIGHT,
    },
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
        if !settings.allowed_programs.is_empty() {
            info!("Allowed programs: {:?}", settings.allowed_programs);
            shared_state.allowed_programs = Some(settings.allowed_programs.clone());
        }
        shared_state.block_kb_on_controller = settings.block_kb_on_controller;

        Ok(())
    }

    fn start(&mut self, _: &tauri::AppHandle) -> Result<(), String> {
        // Controller Input Polling
        thread::spawn(|| {
            sdl3::hint::set("SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
            let sdl_context = sdl3::init().unwrap();
            let gamepad_system = sdl_context.gamepad().unwrap();
            let gamepads = gamepad_system.gamepads().unwrap();

            let expected_id = "0300fa67c82d00000631000014017801";
            info!("expected_id: {}", expected_id);

            let mut list: Vec<SdlInput> = Vec::new();
            list.push(SdlInput::Button(sdl3::gamepad::Button::North));
            list.push(SdlInput::Button(sdl3::gamepad::Button::South));
            list.push(SdlInput::Axis(sdl3::gamepad::Axis::TriggerLeft));

            let x = SdlInput::Axis(sdl3::gamepad::Axis::LeftX);
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
                    // Translate real gamepad inputs to virtual gamepad inputs
                    let mut face_buttons: u16 = 0;
                    let thumb_lx = pad.axis(sdl3::gamepad::Axis::LeftX);
                    let thumb_ly = pad.axis(sdl3::gamepad::Axis::LeftY);
                    let thumb_rx = pad.axis(sdl3::gamepad::Axis::RightX);
                    let thumb_ry = pad.axis(sdl3::gamepad::Axis::RightY);
                    let mut left_trigger = pad.axis(sdl3::gamepad::Axis::TriggerLeft) as u8;
                    let mut right_trigger = pad.axis(sdl3::gamepad::Axis::TriggerRight) as u8;

                    for input in SDL_TO_VIGEM_BUTTON.iter() {
                        if pad.button(input.sdl) {
                            face_buttons = face_buttons | input.vigem as u16;
                        }
                    }

                    info!("face_buttons: {:?} {:?}", face_buttons, thumb_lx);

                    loop {
                        gamepad_system.update();
                        let mut all_pressed = true;
                        for button in list.iter() {
                            match button {
                                SdlInput::Button(b) => {
                                    info!("button: {:?} {:?}", *b, pad.button(*b));
                                    all_pressed &= pad.button(*b);
                                }
                                SdlInput::Axis(a) => {
                                    info!("axis: {:?}", pad.axis(*a));
                                    all_pressed &= pad.axis(*a) > 0;
                                }
                            }
                        }
                        if all_pressed {
                            info!("all_pressed: {:?}", all_pressed);
                        }

                        thread::sleep(std::time::Duration::from_millis(1000));
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
        let shared_state = SHARED_STATE.read().unwrap();
        shared_state.hook_handle.is_some() || shared_state.window_hook_handle.is_some()
    }
}
