#![cfg(target_os = "windows")]

use asr::sync::Mutex;
use log::{debug, error, info};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::LazyLock;
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

static CHANNEL: LazyLock<(
    std::sync::mpsc::Sender<VigemInput>,
    Arc<Mutex<std::sync::mpsc::Receiver<VigemInput>>>,
)> = LazyLock::new(|| {
    let (tx, rx) = channel::<VigemInput>();
    (tx, Arc::new(Mutex::new(rx)))
});

static MASHING_BUTTONS: Lazy<Arc<RwLock<Vec<VigemInput>>>> =
    Lazy::new(|| Arc::new(RwLock::new(Vec::new())));

enum VigemInput {
    Button(u16, bool),
    LeftTrigger(u8),
    RightTrigger(u8),
    LeftX(i16),
    LeftY(i16),
    RightX(i16),
    RightY(i16),
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
        mashing_key_list.push(VigemInput::Button(vigem_client::XButtons::Y as u16, true));
        mashing_key_list.push(VigemInput::Button(vigem_client::XButtons::B as u16, true));
        mashing_key_list.push(VigemInput::LeftTrigger(u8::MAX));
        *MASHING_BUTTONS.write().unwrap() = mashing_key_list;

        // mashing thread
        if MASHING_BUTTONS.read().unwrap().len() == MAX_MASHING_KEY_COUNT as usize {
            info!("SPAWNING MASHER THREAD");
            thread::spawn(|| {
                let transmitter = CHANNEL.0.clone();

                text_masher(|key_to_press| {
                    {
                        if key_to_press > MAX_MASHING_KEY_COUNT {
                            // stop mashing
                            for input in MASHING_BUTTONS.read().unwrap().iter() {
                                match input {
                                    VigemInput::Button(b, _) => {
                                        transmitter.send(VigemInput::Button(*b, false)).unwrap()
                                    }
                                    VigemInput::LeftTrigger(_) => {
                                        transmitter.send(VigemInput::LeftTrigger(0)).unwrap()
                                    }
                                    VigemInput::RightTrigger(_) => {
                                        transmitter.send(VigemInput::RightTrigger(0)).unwrap()
                                    }
                                    _ => error!("Invalid mashing input configured"),
                                }
                            }
                        } else {
                            for (i, input) in MASHING_BUTTONS.read().unwrap().iter().enumerate() {
                                if (i as u8) == key_to_press {
                                    match input {
                                        VigemInput::Button(b, _) => {
                                            transmitter.send(VigemInput::Button(*b, true)).unwrap()
                                        }
                                        VigemInput::LeftTrigger(_) => transmitter
                                            .send(VigemInput::LeftTrigger(u8::MAX))
                                            .unwrap(),
                                        VigemInput::RightTrigger(_) => transmitter
                                            .send(VigemInput::RightTrigger(u8::MAX))
                                            .unwrap(),
                                        _ => error!("Invalid mashing input configured"),
                                    }
                                }
                                if (i as u8) == (key_to_press + 1) % MAX_MASHING_KEY_COUNT {
                                    match input {
                                        VigemInput::Button(b, _) => {
                                            transmitter.send(VigemInput::Button(*b, false)).unwrap()
                                        }
                                        VigemInput::LeftTrigger(_) => {
                                            transmitter.send(VigemInput::LeftTrigger(0)).unwrap()
                                        }
                                        VigemInput::RightTrigger(_) => {
                                            transmitter.send(VigemInput::RightTrigger(0)).unwrap()
                                        }
                                        _ => error!("Invalid mashing input configured"),
                                    }
                                }
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
            let polling_transmitter = CHANNEL.0.clone();

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
                    let mut gamepad_state = vigem_client::XGamepad::default();

                    let mut face_buttons: u16 = 0;
                    let mut thumb_lx = 0;
                    let mut thumb_ly = 0;
                    let mut thumb_rx = 0;
                    let mut thumb_ry = 0;
                    let mut left_trigger = 0;
                    let mut right_trigger = 0;

                    loop {
                        gamepad_system.update();

                        let mut new_face_buttons: u16 = 0;
                        let new_thumb_lx = pad.axis(sdl3::gamepad::Axis::LeftX);
                        let new_thumb_ly = pad.axis(sdl3::gamepad::Axis::LeftY).saturating_neg();
                        let new_thumb_rx = pad.axis(sdl3::gamepad::Axis::RightX);
                        let new_thumb_ry = pad.axis(sdl3::gamepad::Axis::RightY).saturating_neg();
                        let new_left_trigger =
                            (pad.axis(sdl3::gamepad::Axis::TriggerLeft) / 128) as u8;
                        let new_right_trigger =
                            (pad.axis(sdl3::gamepad::Axis::TriggerRight) / 128) as u8;

                        // Translate real gamepad inputs to virtual gamepad inputs
                        for (sdl_button, vigem_button) in BUTTON_MAPPINGS.iter() {
                            if pad.button(*sdl_button) {
                                new_face_buttons = new_face_buttons | *vigem_button as u16;
                            }
                        }

                        // Detect Mashing Buttons
                        let mut all_pressed = true;
                        if MASHING_BUTTONS.read().unwrap().len() != 0 {
                            for button in MASHING_BUTTONS.read().unwrap().iter() {
                                match button {
                                    VigemInput::Button(b, _) => {
                                        all_pressed &= (new_face_buttons & b) > 0;
                                    }
                                    VigemInput::LeftTrigger(_) => {
                                        all_pressed &= new_left_trigger > 0;
                                    }
                                    VigemInput::RightTrigger(_) => {
                                        all_pressed &= new_right_trigger > 0;
                                    }
                                    _ => error!("Invalid mashing input configured"),
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

                        // update virtual gamepad state
                        let mut mask: u16 = 1;
                        for _ in 0..u16::BITS {
                            if (new_face_buttons & mask != 0) ^ (face_buttons & mask != 0) {
                                let is_mashing_button = MASHING_BUTTONS.read().unwrap().iter().any(|item| {
                                    matches!(item, VigemInput::Button(val, _) if *val == mask)
                                });
                                if !(is_mashing_button && all_pressed) {
                                    polling_transmitter
                                        .send(VigemInput::Button(
                                            mask,
                                            new_face_buttons & mask != 0,
                                        ))
                                        .unwrap();
                                }
                            }
                            mask <<= 1;
                        }
                        face_buttons = new_face_buttons;

                        if left_trigger != new_left_trigger {
                            let is_mashing_button = MASHING_BUTTONS
                                .read()
                                .unwrap()
                                .iter()
                                .any(|item| matches!(item, VigemInput::LeftTrigger(_)));
                            if !(is_mashing_button && all_pressed) {
                                polling_transmitter
                                    .send(VigemInput::LeftTrigger(new_left_trigger))
                                    .unwrap();
                                left_trigger = new_left_trigger;
                            }
                        }
                        if right_trigger != new_right_trigger {
                            let is_mashing_button = MASHING_BUTTONS
                                .read()
                                .unwrap()
                                .iter()
                                .any(|item| matches!(item, VigemInput::RightTrigger(_)));
                            if !(is_mashing_button && all_pressed) {
                                polling_transmitter
                                    .send(VigemInput::RightTrigger(new_right_trigger))
                                    .unwrap();
                                right_trigger = new_right_trigger;
                            }
                        }
                        if thumb_lx != new_thumb_lx {
                            polling_transmitter
                                .send(VigemInput::LeftX(new_thumb_lx))
                                .unwrap();
                            thumb_lx = new_thumb_lx;
                        }
                        if thumb_ly != new_thumb_ly {
                            polling_transmitter
                                .send(VigemInput::LeftY(new_thumb_ly))
                                .unwrap();
                            thumb_ly = new_thumb_ly;
                        }
                        if thumb_rx != new_thumb_rx {
                            polling_transmitter
                                .send(VigemInput::RightX(new_thumb_rx))
                                .unwrap();
                            thumb_rx = new_thumb_rx;
                        }
                        if thumb_ry != new_thumb_ry {
                            polling_transmitter
                                .send(VigemInput::RightY(new_thumb_ry))
                                .unwrap();
                            thumb_ry = new_thumb_ry;
                        }

                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                }

                thread::sleep(std::time::Duration::from_millis(1000));
            }
        });

        // virtual controller updates from event queue
        thread::spawn(|| {
            let reciever = Arc::clone(&CHANNEL.1);

            let mut gamepad_state = vigem_client::XGamepad::default();

            loop {
                let message = { reciever.lock().recv() };

                if let Ok(input) = message {
                    match input {
                        VigemInput::Button(val, should_press) => {
                            if should_press {
                                gamepad_state.buttons = XButtons(gamepad_state.buttons.raw | val)
                            } else {
                                gamepad_state.buttons = XButtons(gamepad_state.buttons.raw & !val)
                            }
                        }
                        VigemInput::LeftTrigger(val) => gamepad_state.left_trigger = val,
                        VigemInput::RightTrigger(val) => gamepad_state.right_trigger = val,
                        VigemInput::LeftX(val) => gamepad_state.thumb_lx = val,
                        VigemInput::LeftY(val) => gamepad_state.thumb_ly = val,
                        VigemInput::RightX(val) => gamepad_state.thumb_rx = val,
                        VigemInput::RightY(val) => gamepad_state.thumb_ry = val,
                    }

                    {
                        let mut shared_state = SHARED_STATE.write().unwrap();
                        let mut temp_target = shared_state.target.take();
                        if let Some(ref mut target) = &mut temp_target {
                            let _ = target.update(&gamepad_state);

                            shared_state.target = temp_target;
                        }
                    }
                }
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
