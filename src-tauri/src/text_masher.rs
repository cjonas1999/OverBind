use asr::{
    game_engine::unity::mono::{self, UnityPointer},
    Address, PointerSize, Process,
};
use log::{debug, info};
use once_cell::sync::Lazy;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{thread::sleep, time::Duration};

const TARGET_RATE: f64 = 36.0;
pub static IS_MASHER_ACTIVE: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

struct HKConfig {
    module_name: &'static str,
    base_offset: u32,
    pointer_chain: &'static [u32],
}

static CONFIGS: &[(&str, HKConfig)] = &[
    (
        "hollow_knight.x86_64",
        HKConfig {
            module_name: "libmono.so",
            base_offset: 0x004AAA68,
            pointer_chain: &[0x138, 0x898, 0x20, 0x28, 0x10c],
        },
    ),
    (
        "Hollow Knight.exe",
        HKConfig {
            module_name: "Hollow Knight.exe",
            base_offset: 0x00FB85AC,
            pointer_chain: &[0x20, 0x4, 0x10, 0x4, 0x4, 0x50, 0x38, 0x0],
        },
    ),
    (
        "hollow_knight.exe",
        HKConfig {
            module_name: "hollow_knight.exe",
            base_offset: 0x00FB85AC,
            pointer_chain: &[0x20, 0x4, 0x10, 0x4, 0x4, 0x50, 0x38, 0x0],
        },
    ),
];

fn attach_hollow_knight() -> Option<(Process, &'static str)> {
    CONFIGS
        .iter()
        .find_map(|(name, _config)| Process::attach(name).map(|proc| (proc, *name)))
}

fn get_config(process_name: &str) -> Option<&'static HKConfig> {
    CONFIGS.iter().find_map(|(name, config)| {
        if *name == process_name {
            Some(config)
        } else {
            None
        }
    })
}

fn wait_attach(process: &Process) -> (mono::Module, mono::Image) {
    let mut found_module = false;
    let mut needed_retry = false;
    loop {
        if let Some(module) = mono::Module::attach_auto_detect(process) {
            if !found_module {
                found_module = true;
                info!("GameManagerFinder wait_attach: module get_default_image...");
            }
            for _ in 0..0x10 {
                if let Some(image) = module.get_default_image(process) {
                    info!("GameManagerFinder wait_attach: got module and image");
                    return (module, image);
                }
            }
            if !needed_retry {
                needed_retry = true;
                info!("GameManagerFinder wait_attach: retry...");
            }
        }
    }
}

fn resolve_pointer_chain(
    process: &Process,
    base: Address,
    chain: &[u32],
    ps: PointerSize,
) -> Option<Address> {
    if chain.is_empty() {
        return Some(base);
    }

    let mut addr = process.read_pointer(base, ps).ok()?;
    for &offset in &chain[..chain.len() - 1] {
        addr = process.read_pointer(addr + offset, ps).ok()?;
    }

    Some(addr + chain[chain.len() - 1])
}

pub fn text_masher(do_key_event: impl Fn(bool)) {
    info!("TextMasher starting up");
    let target_interval: Duration = Duration::from_secs_f64(1.0 / TARGET_RATE);
    let mut is_masher_active = false;
    let mut is_key_down = false;

    loop {
        let process = attach_hollow_knight();
        if let Some((process, process_name)) = process {
            info!("Found Hollow Knight: {:?}", process_name);
            let config = match get_config(process_name) {
                Some(cfg) => cfg,
                None => {
                    info!("No config found for {:?}", process_name);
                    continue;
                }
            };

            info!("GameManagerFinder wait_attach...");
            let _ = process.until_closes({
                let (module, image) = wait_attach(&process);

                loop {
                    if IS_MASHER_ACTIVE.load(Ordering::SeqCst) {
                        if let Some(module_address) =
                            process.get_module_address(config.module_name).ok()
                        {
                            let input_pointer: UnityPointer<3> = UnityPointer::new(
                                "GameManager",
                                0,
                                &[
                                    "_instance",
                                    "<inputHandler>k__BackingField",
                                    "acceptingInput",
                                ],
                            );
                            let accepting_input: bool = input_pointer
                                .deref(&process, &module, &image)
                                .unwrap_or_default();

                            if accepting_input {
                                let base: Address = module_address + config.base_offset;
                                if let Some(dialogue_box_addr) = resolve_pointer_chain(
                                    &process,
                                    base,
                                    &config.pointer_chain,
                                    PointerSize::Bit64,
                                ) {
                                    if is_masher_active != IS_MASHER_ACTIVE.load(Ordering::SeqCst) {
                                        is_masher_active = IS_MASHER_ACTIVE.load(Ordering::SeqCst);
                                        is_key_down = false;
                                    }
                                    if matches!(process.read::<u8>(dialogue_box_addr + 0x2E), Ok(is_dialogue_hidden) if is_dialogue_hidden == 0) {
                                        debug!("Trigger do key event: {}", is_masher_active);
                                        do_key_event(is_key_down);
                                        is_key_down = !is_key_down;
                                    }
                                }
                            }
                        } else {
                            info!("Cannot attach to base module address");
                            break;
                        }
                    }

                    sleep(target_interval);
                }
            });
        }
        sleep(target_interval);
    }
}
