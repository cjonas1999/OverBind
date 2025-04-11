use asr::{
    game_engine::unity::mono::{self, UnityPointer},
    Process,
};
use std::{thread::sleep, time::Duration};

static HOLLOW_KNIGHT_NAMES: [&str; 5] = [
    "Hollow Knight.exe",
    "hollow_knight.exe",    // Windows
    "hollow_knight.x86_64", // Linux
    "Hollow Knight",        // Mac
    "hollow_knight",        // Mac
];

fn attach_hollow_knight() -> Option<Process> {
    HOLLOW_KNIGHT_NAMES.into_iter().find_map(Process::attach)
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

pub fn text_masher() {
    loop {
        let process = attach_hollow_knight();
        println!("Searching for Hollow Knight...");
        if let Some(process) = process {
            println!("Found Hollow Knight!");
            let _ = process.until_closes({
                println!("GameManagerFinder wait_attach...");
                //let pointer_size = process_pointer_size(process).unwrap_or(PointerSize::Bit64);
                let (module, image) = wait_attach(&process);

                loop {
                    println!("Getting geo value...");
                    let geo_pointer: UnityPointer<3> =
                        UnityPointer::new("GameManager", 0, &["_instance", "playerData", "geo"]);
                    let geo_value: i32 = geo_pointer
                        .deref(&process, &module, &image)
                        .unwrap_or_default();
                    println!("Geo value: {:?}", geo_value);

                    sleep(Duration::from_millis(100));
                }
            });
        }
    }
}
