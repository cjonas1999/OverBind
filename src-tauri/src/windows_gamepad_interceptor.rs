#![cfg(target_os = "windows")]

use asr::sync::Mutex;
use log::{debug, error, info};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use vigem_client::{Client, XButtons, XGamepad};

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

static MASHING_BUTTONS: Lazy<Arc<RwLock<Vec<VigemInput>>>> =
    Lazy::new(|| Arc::new(RwLock::new(Vec::new())));

enum VigemInput {
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
        mashing_key_list.push(VigemInput::Button(vigem_client::XButtons::A as u16));
        mashing_key_list.push(VigemInput::Button(vigem_client::XButtons::B as u16));
        mashing_key_list.push(VigemInput::Button(vigem_client::XButtons::X as u16));
        // mashing_key_list.push(VigemInput::LeftTrigger);
        *MASHING_BUTTONS.write().unwrap() = mashing_key_list;

        // mashing thread
        if MASHING_BUTTONS.read().unwrap().len() == MAX_MASHING_KEY_COUNT as usize {
            info!("SPAWNING MASHER THREAD");
            thread::spawn(|| {
                text_masher(|key_to_press| {
                    let mut gamepad_state = vigem_client::XGamepad::default();

                    if key_to_press < MAX_MASHING_KEY_COUNT {
                        let mash_buttons = MASHING_BUTTONS.read().unwrap();
                        if let Some(press) = mash_buttons.get(key_to_press as usize) {
                            match press {
                                VigemInput::Button(b) => {
                                    gamepad_state.buttons = vigem_client::XButtons(*b)
                                }
                                VigemInput::LeftTrigger => gamepad_state.left_trigger = u8::MAX,
                                VigemInput::RightTrigger => gamepad_state.right_trigger = u8::MAX,
                            }
                        }
                    }

                    {
                        let mut shared_state = SHARED_STATE.write().unwrap();
                        let mut temp_target = shared_state.target.take();
                        if let Some(ref mut target) = &mut temp_target {
                            let _ = target.update(&gamepad_state);

                            shared_state.target = temp_target;
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

            loop {
                // Identify gamepad with correct id
                let pad_opt = gamepads.iter().find(|&joystick_id| {
                    gamepad_system.guid_for_id(*joystick_id).string() == expected_id
                });
                if let Ok(pad) = gamepad_system.open(*pad_opt.unwrap()) {
                    loop {
                        gamepad_system.update();

                        let mut face_buttons: u16 = 0;

                        // Translate held buttons to vigem-style u16 bitflags
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
                                    VigemInput::Button(b) => {
                                        all_pressed &= (face_buttons & b) > 0;
                                    }
                                    VigemInput::LeftTrigger => {
                                        all_pressed &=
                                            pad.axis(sdl3::gamepad::Axis::TriggerRight) > 0;
                                    }
                                    VigemInput::RightTrigger => {
                                        all_pressed &=
                                            pad.axis(sdl3::gamepad::Axis::TriggerRight) > 0;
                                    }
                                }
                            }
                        } else {
                            //no mashing keys configured
                            all_pressed = false
                        }

                        if IS_MASHER_ACTIVE.load(Ordering::SeqCst) != all_pressed {
                            debug!("all mashing triggers pressed: {}", all_pressed);
                            IS_MASHER_ACTIVE.store(all_pressed, Ordering::SeqCst);
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
