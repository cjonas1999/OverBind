#![cfg(target_os = "macos")]

use core_foundation::base::CFAllocatorRef;
use core_foundation::runloop::kCFRunLoopCommonModes;
use core_foundation::runloop::kCFRunLoopDefaultMode;
use core_foundation::runloop::CFRunLoop;
use core_graphics::display::CFDictionaryRef;
use core_graphics::event::CGEvent;
use core_graphics::event::CGEventFlags;
use core_graphics::event::CGEventTap;
use core_graphics::event::CGEventTapLocation;
use core_graphics::event::CGEventTapOptions;
use core_graphics::event::CGEventTapPlacement;
use core_graphics::event::CGEventTapProxy;
use core_graphics::event::CGEventType;
use core_graphics::event::EventField;
use core_graphics::event_source::CGEventSource;
use core_graphics::event_source::CGEventSourceStateID;
use io_kit_sys::ret::IOReturn;
use io_kit_sys::types::IOOptionBits;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::cmp;
use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::key_interceptor::KeyInterceptorTrait;
use crate::{get_config_path, Settings};

pub type IOHIDUserDeviceRef = *mut c_void;
extern "C" {
    pub fn IOHIDUserDeviceCreateWithProperties(
        allocator: CFAllocatorRef,
        properties: CFDictionaryRef,
        options: IOOptionBits,
    ) -> IOHIDUserDeviceRef;

    pub fn IOHIDUserDeviceHandleReportWithTimeStamp(
        device: IOHIDUserDeviceRef,
        timestamp: u64,
        report: *const u8,
        report_length: usize,
    ) -> IOReturn;
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
    result_value: i64,
}

#[derive(Clone)]
struct OppositeKey {
    is_pressed: bool,
    is_virtual_pressed: bool,
    opposite_key_type: String,
    opposite_key_value: i64,
    opposite_key_mapping: Option<i64>,
}

static KEY_STATES: Lazy<Arc<RwLock<HashMap<i64, KeyState>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static OPPOSITE_KEY_STATES: Lazy<Arc<RwLock<HashMap<i64, OppositeKey>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static OPPOSITE_KEY_MAPPINGS: Lazy<Arc<RwLock<HashMap<i64, i64>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static DPAD_BUTTON_STATES: Lazy<Arc<RwLock<HashMap<u32, KeyState>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

struct SharedState {
    device: Option<IOHIDUserDeviceRef>,
    event_source: Option<CGEventSource>,
    allowed_programs: Option<Vec<String>>,
    active_app_name: Option<String>,
    block_kb_on_controller: bool,
}

unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

static SHARED_STATE: Lazy<Arc<RwLock<SharedState>>> = Lazy::new(|| {
    Arc::new(RwLock::new(SharedState {
        device: None,
        event_source: None,
        allowed_programs: None,
        active_app_name: None,
        block_kb_on_controller: false,
    }))
});

static SHOULD_RUN: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

pub(crate) struct MacKeyInterceptor {}

impl KeyInterceptorTrait for MacKeyInterceptor {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self {}
    }

    fn initialize(&mut self, settings: &Settings) -> Result<(), String> {
        let event_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .expect("Failed to create CGEventSource");

        let mut shared_state = SHARED_STATE.write().unwrap();
        shared_state.event_source = Some(event_source);

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

        // const GAMEPAD_HID_DESCRIPTOR: &[u8] = &[
        //     0x05, 0x01, // Usage Page (Generic Desktop Controls)
        //     0x09, 0x05, // Usage (Gamepad)
        //     0xA1, 0x01, // Collection (Application)
        //     0x85, 0x01, //    Report ID (1)
        //     // Buttons (16 buttons total)
        //     0x05, 0x09, //    Usage Page (Button)
        //     0x19, 0x01, //    Usage Minimum (1)
        //     0x29, 0x10, //    Usage Maximum (16)
        //     0x15, 0x00, //    Logical Minimum (0)
        //     0x25, 0x01, //    Logical Maximum (1)
        //     0x95, 0x10, //    Report Count (16)
        //     0x75, 0x01, //    Report Size (1)
        //     0x81, 0x02, //    Input (Data, Var, Abs)
        //     // Padding (to align byte boundaries after buttons)
        //     0x75, 0x02, //    Report Size (2)
        //     0x95, 0x01, //    Report Count (1)
        //     0x81, 0x03, //    Input (Const, Var, Abs)
        //     // Left Stick X and Y Axes
        //     0x05, 0x01, //    Usage Page (Generic Desktop Controls)
        //     0x09, 0x30, //    Usage (X)
        //     0x09, 0x31, //    Usage (Y)
        //     0x15, 0x81, //    Logical Minimum (-127)
        //     0x25, 0x7F, //    Logical Maximum (127)
        //     0x75, 0x08, //    Report Size (8)
        //     0x95, 0x02, //    Report Count (2)
        //     0x81, 0x02, //    Input (Data, Var, Abs)
        //     // Right Stick X and Y Axes
        //     0x05, 0x01, //    Usage Page (Generic Desktop Controls)
        //     0x09, 0x32, //    Usage (Rx) -> Right Stick X
        //     0x09, 0x33, //    Usage (Ry) -> Right Stick Y
        //     0x15, 0x81, //    Logical Minimum (-127)
        //     0x25, 0x7F, //    Logical Maximum (127)
        //     0x75, 0x08, //    Report Size (8)
        //     0x95, 0x02, //    Report Count (2)
        //     0x81, 0x02, //    Input (Data, Var, Abs)
        //     // Triggers (Left and Right)
        //     0x05, 0x02, //    Usage Page (Simulation Controls)
        //     0x09, 0x02, //    Usage (LT) -> Left Trigger
        //     0x09, 0x03, //    Usage (RT) -> Right Trigger
        //     0x15, 0x00, //    Logical Minimum (0)
        //     0x25, 0xFF, //    Logical Maximum (255)
        //     0x75, 0x08, //    Report Size (8)
        //     0x95, 0x02, //    Report Count (2)
        //     0x81, 0x02, //    Input (Data, Var, Abs)
        //     0xC0, // End Collection
        // ];

        // let allocator: CFAllocatorRef = unsafe { kCFAllocatorDefault };
        // let mut properties = CFMutableDictionary::new();
        // let vendor_id = CFNumber::from(0x045e);
        // properties.set(
        //     CFString::new("VendorID").as_CFType(),
        //     vendor_id.as_CFType(),
        // );
        // let product_id = CFNumber::from(0x028e);
        // properties.set(
        //     CFString::new("ProductID").as_CFType(),
        //     product_id.as_CFType(),
        // );
        // let hid_descriptor = CFData::from_buffer(GAMEPAD_HID_DESCRIPTOR);
        // properties.set(
        //     CFString::new("HIDDescriptor").as_CFType(),
        //     hid_descriptor.as_CFType(),
        // );

        // let device = unsafe { IOHIDUserDeviceCreateWithProperties(allocator, properties.as_concrete_TypeRef(), 0) };

        // if device.is_null() {
        //     println!("Failed to create HID device");
        //     return Err("Failed to create HID device".to_owned());
        // }
        // {
        //     let mut shared_state = SHARED_STATE.write().unwrap();
        //     shared_state.device = Some(device);
        // }

        let data: Vec<ConfigJsonData> =
            serde_json::from_str(&contents).map_err(|e| e.to_string())?;
        let mut key_states = HashMap::new();
        let mut opposite_key_states = HashMap::new();
        let mut opposite_key_mappings = HashMap::new();
        let mut dpad_button_states = HashMap::new();

        for item in &data {
            let windows_keycode =
                u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");
            let keycode = windows_code_to_mac_keycode(windows_keycode).unwrap();

            if item.result_type != "socd" {
                let mut result_value = item.result_value as i64;
                if item.result_type == "keyboard" {
                    result_value = windows_code_to_mac_keycode(result_value as u32).unwrap();
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
                            result_value: opposite_dpad as i64,
                        });
                    println!(
                        "DPad button code: {:?}, Opposite dpad button code: {:?}",
                        item.result_value, opposite_dpad
                    )
                }

                let key_state = key_states.entry(keycode).or_insert_with(|| KeyState {
                    is_pressed: false,
                    result_type: item.result_type.clone(),
                    result_value: result_value,
                });
                println!(
                    "Keycode: {:?}, ResultType: {:?}, ResultValue {:?}",
                    keycode, key_state.result_type, key_state.result_value
                );
            }
        }

        for item in &data {
            let windows_keycode =
                u32::from_str_radix(&item.keycode, 16).expect("Invalid hexadecimal string");
            let keycode = windows_code_to_mac_keycode(windows_keycode).unwrap();

            if item.result_type == "socd" {
                let windows_opposite_keycode = item.result_value as u16;
                let opposite_keycode =
                    windows_code_to_mac_keycode(windows_opposite_keycode.into()).unwrap();

                let this_key_state_mapping = key_states.get(&keycode.clone());
                let opposite_key_state_mapping = key_states.get(&opposite_keycode.clone());
                let mut key_type = String::from("keyboard");
                let mut opposite_key_mapping = None;

                if this_key_state_mapping.is_some()
                    && (this_key_state_mapping.unwrap().result_type == "keyboard")
                {
                    opposite_key_mappings.insert(
                        keycode.clone(),
                        this_key_state_mapping.unwrap().result_value as i64,
                    );
                } else {
                    opposite_key_mappings.insert(keycode.clone(), keycode.clone());
                }

                if opposite_key_state_mapping.is_some()
                    && (opposite_key_state_mapping.unwrap().result_type == "keyboard"
                        || opposite_key_state_mapping.unwrap().result_type == "face_button")
                {
                    key_type = opposite_key_state_mapping.unwrap().result_type.clone();
                    opposite_key_mapping = Some(opposite_key_state_mapping.unwrap().result_value);
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

                println!(
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

        // Start listening for key events
        println!("Spawning key event listener thread");
        thread::spawn(move || {
            let current = CFRunLoop::get_current();
            let tap = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                vec![CGEventType::KeyDown, CGEventType::KeyUp],
                handle_key_event,
            )
            .expect("Failed to create event tap");

            unsafe {
                let loop_source = tap
                    .mach_port
                    .create_runloop_source(0)
                    .expect("Failed to define loop source");
                current.add_source(&loop_source, kCFRunLoopCommonModes);
                tap.enable();
                let mut should_run = SHOULD_RUN.load(Ordering::SeqCst);
                loop {
                    let new_should_run = SHOULD_RUN.load(Ordering::SeqCst);

                    if new_should_run != should_run {
                        // Detect transition in SHOULD_RUN
                        should_run = new_should_run;
                        if should_run {
                            println!("Starting run loop...");
                        } else {
                            println!("Stopping run loop...");
                            CFRunLoop::stop(&current); // Stop the run loop instantly
                        }
                    }

                    if should_run {
                        // Run the loop for a short time, then check SHOULD_RUN again
                        CFRunLoop::run_in_mode(
                            kCFRunLoopDefaultMode,
                            Duration::from_millis(100),
                            true,
                        );
                    } else {
                        // If not running, sleep briefly to avoid busy-waiting
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        });

        Ok(())
    }

    fn stop(&self, _app: &tauri::AppHandle) {
        SHOULD_RUN.store(false, Ordering::SeqCst);
    }

    fn is_running(&self) -> bool {
        SHOULD_RUN.load(Ordering::SeqCst)
    }
}

fn handle_key_event(
    _proxy: CGEventTapProxy,
    _event_type: CGEventType,
    event: &CGEvent,
) -> Option<CGEvent> {
    println!(
        "Key pressed: {:?}",
        event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
    );
    let key_code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
    let key_is_down = match event.get_type() {
        CGEventType::KeyDown => true,
        CGEventType::KeyUp => false,
        _ => return None,
    };

    if event.get_flags().contains(CGEventFlags::CGEventFlagHelp) {
        println!("Virtual event");
        // Remove the help flag
        let mut flags = event.get_flags();
        flags.remove(CGEventFlags::CGEventFlagHelp);
        event.set_flags(flags);
        return Some(event.to_owned()); // Return the event without the help flag
    }

    // Update Key State
    {
        let mut key_states = KEY_STATES.write().unwrap();
        match key_states.get_mut(&key_code) {
            Some(state) => state.is_pressed = key_is_down,
            _ => (),
        }
    }
    println!("Keycode: {:?}, Key is down: {:?}", key_code, key_is_down);

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
                println!("Releasing opposite key");
                opposite_key_state.is_virtual_pressed = false;
                send_new_keyboard_event(cloned_key_state.opposite_key_value, false);
            } else if !key_is_down && opposite_key_state.is_pressed {
                println!("Repressing opposite key");
                opposite_key_state.is_virtual_pressed = true;
                send_new_keyboard_event(cloned_key_state.opposite_key_value, true);
            }
        }
    }

    // Rebinds
    {
        let key_states = KEY_STATES.read().unwrap();
        if let Some(key_state) = key_states.get(&key_code) {
            println!("Key state: {:?}", key_state.result_value);
            if key_state.result_type == "keyboard" {
                send_keyboard_event(event, key_state.result_value, key_is_down);
                println!(
                    "Sending keyboard event {:?}",
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
                );
                return Some(event.to_owned());
            }
        }
    }

    // Controller

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

    let report = vec![
        1,
        (face_buttons & 0xFF) as u8,
        (face_buttons >> 8) as u8,
        thumb_lx as u8,
        thumb_ly as u8,
        thumb_rx as u8,
        thumb_ry as u8,
        left_trigger,
        right_trigger,
    ];
    {
        // let device = SHARED_STATE.read().unwrap().device.clone().unwrap();
        // let result = unsafe {
        //     IOHIDUserDeviceHandleReportWithTimeStamp(
        //         device,
        //         0,               // Timestamp (use 0 for "now")
        //         report.as_ptr(), // Pointer to the report data
        //         report.len(),    // Length of the report
        //     )
        // };
        // if result != kIOReturnSuccess {
        //     println!("Failed to send gamepad report");
        // }
    }

    if SHARED_STATE.read().unwrap().block_kb_on_controller {
        event.set_type(CGEventType::Null);
        return Some(event.to_owned());
    }

    Some(event.to_owned())
}

fn send_keyboard_event(event: &CGEvent, key_code: i64, key_is_down: bool) {
    let event_type = if key_is_down {
        CGEventType::KeyDown
    } else {
        CGEventType::KeyUp
    };
    event.set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, key_code);
    event.set_type(event_type);
}

fn send_new_keyboard_event(key_code: i64, key_is_down: bool) {
    let event_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .expect("Failed to create event source");
    let new_event = CGEvent::new_keyboard_event(event_source, key_code as u16, key_is_down)
        .expect("Failed to create second key event");
    new_event.set_flags(CGEventFlags::CGEventFlagHelp);
    new_event.post(CGEventTapLocation::HID);
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

fn windows_code_to_mac_keycode(code: u32) -> Option<i64> {
    match code {
        0x08 => Some(51),
        0x09 => Some(48),
        0x0D => Some(36),
        0x10 => Some(57),
        0x11 => Some(59),
        0x12 => Some(58),
        0x14 => Some(57),
        0x1B => Some(53),
        0x20 => Some(49),
        0x25 => Some(123),
        0x26 => Some(126),
        0x27 => Some(124),
        0x28 => Some(125),
        0x2E => Some(51),
        0x30 => Some(29),
        0x31 => Some(18),
        0x32 => Some(19),
        0x33 => Some(20),
        0x34 => Some(21),
        0x35 => Some(23),
        0x36 => Some(22),
        0x37 => Some(26),
        0x38 => Some(28),
        0x39 => Some(25),
        0x41 => Some(0),
        0x42 => Some(11),
        0x43 => Some(8),
        0x44 => Some(2),
        0x45 => Some(14),
        0x46 => Some(3),
        0x47 => Some(5),
        0x48 => Some(4),
        0x49 => Some(34),
        0x4A => Some(38),
        0x4B => Some(40),
        0x4C => Some(37),
        0x4D => Some(46),
        0x4E => Some(45),
        0x4F => Some(31),
        0x50 => Some(35),
        0x51 => Some(12),
        0x52 => Some(15),
        0x53 => Some(1),
        0x54 => Some(17),
        0x55 => Some(32),
        0x56 => Some(9),
        0x57 => Some(13),
        0x58 => Some(7),
        0x59 => Some(16),
        0x5A => Some(6),
        0x5B => Some(55),
        0x5C => Some(55),
        0x70 => Some(122),
        0x71 => Some(120),
        0x72 => Some(99),
        0x73 => Some(118),
        0x74 => Some(96),
        0x75 => Some(97),
        0x76 => Some(98),
        0x77 => Some(100),
        0x78 => Some(101),
        0x79 => Some(109),
        0x7A => Some(103),
        0x7B => Some(111),
        0xA0 => Some(57),
        0xA1 => Some(60),
        0xA2 => Some(59),
        0xA4 => Some(58),
        0xA5 => Some(61),
        _ => None,
    }
}
