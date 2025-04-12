use asr::{
    game_engine::unity::mono::{self, UnityPointer},
    Address, PointerSize, Process,
};
use std::{thread::sleep, time::Duration};

use crate::livesplit_core::process_get_module_address;

static HOLLOW_KNIGHT_NAMES: [&str; 5] = [
    "Hollow Knight.exe",
    "hollow_knight.exe",    // Windows
    "hollow_knight.x86_64", // Linux
    "Hollow Knight",        // Mac
    "hollow_knight",        // Mac
];

fn attach_hollow_knight() -> Option<(Process, &'static str)> {
    HOLLOW_KNIGHT_NAMES
        .into_iter()
        .find_map(|name| Process::attach(name).map(|proc| (proc, name)))
}

fn wait_attach(process: &Process) -> (mono::Module, mono::Image) {
    let mut found_module = false;
    let mut needed_retry = false;
    loop {
        if let Some(module) = mono::Module::attach_auto_detect(process) {
            if !found_module {
                found_module = true;
                println!("GameManagerFinder wait_attach: module get_default_image...");
            }
            for _ in 0..0x10 {
                if let Some(image) = module.get_default_image(process) {
                    println!("GameManagerFinder wait_attach: got module and image");
                    return (module, image);
                }
            }
            if !needed_retry {
                needed_retry = true;
                println!("GameManagerFinder wait_attach: retry...");
            }
        } else {
            println!("GameManagerFinder failed to attach");
        }
    }
}

fn is_dialogue_box_hidden(process: &Process, base_address: Address) -> Option<bool> {
    let ps = PointerSize::Bit64;
    let mut addr = process.read_pointer(base_address, ps).ok()?;
    addr = process.read_pointer(addr + 0x20, ps).ok()?;
    addr = process.read_pointer(addr + 0x4, ps).ok()?;
    addr = process.read_pointer(addr + 0x10, ps).ok()?;
    addr = process.read_pointer(addr + 0x4, ps).ok()?;
    addr = process.read_pointer(addr + 0x4, ps).ok()?;
    addr = process.read_pointer(addr + 0x50, ps).ok()?;
    let dialogue_box_addr = process.read_pointer(addr + 0x38, ps).ok()?;

    let hidden_val = process.read::<u8>(dialogue_box_addr + 0x2E).ok()?; // Read the `hidden` field
    Some(hidden_val != 0)
}

pub fn text_masher() {
    loop {
        let process = attach_hollow_knight();
        println!("Searching for Hollow Knight...");
        if let Some((process, process_name)) = process {
            println!("Found Hollow Knight!");
            let _ = process.until_closes({
                println!("GameManagerFinder wait_attach...");
                //let pointer_size = process_pointer_size(process).unwrap_or(PointerSize::Bit64);
                let (module, image) = wait_attach(&process);

                loop {
                    if let Some(module_address) = process.get_module_address(process_name).ok() {
                        let base = module_address + 0x00FB85AC;
                        let dialogue: &'static str = match is_dialogue_box_hidden(&process, base) {
                            Some(true) => "false",
                            Some(false) => "true",
                            None => "null",
                        };
                        println!("Dialogue: {:?}", dialogue);
                    } else {
                        println!("Cannot attach to base module address");
                    }

                    sleep(Duration::from_millis(100));
                }
            });
        }
    }
}
