use asr::{
    game_engine::unity::mono::{self, UnityPointer},
    Address, PointerSize, Process,
};
use once_cell::sync::Lazy;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use std::{thread::sleep, time::Duration};

pub const MAX_MASHING_KEY_COUNT: u8 = 3;

const TARGET_RATE: f64 = 37.0;
pub static IS_MASHER_ACTIVE: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));
pub static SHOULD_TERMINATE_MASHER: Lazy<Arc<AtomicBool>> =
    Lazy::new(|| Arc::new(AtomicBool::new(false)));

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
            base_offset: 0x004ABF88,
            pointer_chain: &[0xa0, 0x268, 0x8, 0x10, 0x70, 0x20, 0x10c],
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

fn wait_attach_until_close<F, R>(f: F)
where
    F: Fn(&Process, &mono::Module, &mono::Image, &HKConfig) -> R,
{
    let process_opt = attach_hollow_knight();
    if let None = process_opt {
        sleep(std::time::Duration::from_millis(500));
        return;
    }

    let (process, process_name) = process_opt.unwrap();
    log::info!("Found Hollow Knight: {:?}", process_name);

    let config = match get_config(process_name) {
        Some(cfg) => cfg,
        None => {
            log::info!("No config found for {:?}", process_name);
            return;
        }
    };

    let mut found_module = false;
    loop {
        if let Some(module) = mono::Module::attach_auto_detect(&process) {
            if !found_module {
                found_module = true;
                log::info!("GameManagerFinder wait_attach: module get_default_image...");
            }
            for _ in 0..0x10 {
                if let Some(image) = module.get_default_image(&process) {
                    log::info!("GameManagerFinder wait_attach: got module and image");
                    f(&process, &module, &image, &config);
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
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

pub fn text_masher(
    do_key_event: impl Fn(u8),
    toggle_overlay: impl Fn(bool) -> Result<(), Box<dyn std::error::Error>>,
) {
    log::info!("TextMasher starting up");
    let target_interval: Duration = Duration::from_secs_f64(1.0 / TARGET_RATE);
    IS_MASHER_ACTIVE.store(false, Ordering::SeqCst);
    SHOULD_TERMINATE_MASHER.store(false, Ordering::SeqCst);

    'mainloop: loop {
        if SHOULD_TERMINATE_MASHER.load(Ordering::SeqCst) {
            let _ = toggle_overlay(false);
            break 'mainloop;
        }

        log::info!("GameManagerFinder wait_attach...");
        let _ = wait_attach_until_close(|process, module, image, config| {
            loop {
                if SHOULD_TERMINATE_MASHER.load(Ordering::SeqCst) {
                    let res = toggle_overlay(false);
                    if res.is_err() {
                        log::error!("Failed to toggle masher overlay");
                    }
                    log::info!("Breaking mainloop in masher thread");
                    return;
                }

                if IS_MASHER_ACTIVE.load(Ordering::SeqCst) {
                    if let Some(err) = toggle_overlay(false).err() {
                        log::error!("Failed to toggle masher overlay: {}", err);
                        continue;
                    }
                    let Ok(module_address) = process.get_module_address(config.module_name) else {
                        log::info!("Cannot attach to base module address");
                        break;
                    };

                    let base: Address = module_address + config.base_offset;
                    let dialogue_box_opt = resolve_pointer_chain(
                        &process,
                        base,
                        &config.pointer_chain,
                        PointerSize::Bit64,
                    );

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
                        if let Some(dialogue_box_addr) = dialogue_box_opt {
                            let mut next_time = Instant::now() + target_interval;
                            let mut key_to_press = 0;

                            if IS_MASHER_ACTIVE.load(Ordering::SeqCst)
                                && matches!(process.read::<u8>(dialogue_box_addr + 0x2E), Ok(is_dialogue_hidden) if is_dialogue_hidden == 0)
                            {
                                do_key_event(100); // release keys
                                while IS_MASHER_ACTIVE.load(Ordering::SeqCst)
                                    && matches!(process.read::<u8>(dialogue_box_addr + 0x2E), Ok(is_dialogue_hidden) if is_dialogue_hidden == 0)
                                {
                                    let _ = toggle_overlay(true);
                                    log::debug!("Trigger do key event: {}", key_to_press);
                                    do_key_event(key_to_press);
                                    key_to_press = (key_to_press + 1) % MAX_MASHING_KEY_COUNT;

                                    // Calculate and wait for next interval
                                    let now = Instant::now();
                                    next_time += target_interval;
                                    if next_time > now {
                                        sleep(next_time - now);
                                    }
                                }
                                do_key_event(100);
                                let _ = toggle_overlay(false);
                            }
                        } else {
                            log::debug!("dialogue box not found");
                        }
                    }
                }
                sleep(Duration::from_millis(100));
            }
        });
    }
}
