use lazy_static::lazy_static;
#[cfg(target_os = "windows")]
use libc::{c_char, c_int, c_uint, c_ulong};
#[cfg(target_os = "windows")]
use windows::Win32::System::Diagnostics::ToolHelp::{
    Module32FirstW, Module32NextW, Process32FirstW, Process32NextW, MODULEENTRY32W,
    PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
};
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Diagnostics::Debug::ReadProcessMemory,
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    },
    System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
};

#[cfg(target_os = "linux")]
use libc::{c_char, c_int, c_uint, c_ulong, iovec, process_vm_readv};

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

lazy_static! {
    static ref PROCESS_LIST: Mutex<ProcessList> = Mutex::new(ProcessList::new());
}

#[no_mangle]
pub unsafe extern "C" fn process_attach(ptr: *const c_char, len: c_uint) -> i32 {
    if ptr.is_null() {
        println!("Error: Null pointer received for process name.");
        return 0;
    }
    // Convert the C string to a Rust string using the given length
    let process_name = match std::slice::from_raw_parts(ptr as *const u8, len as usize) {
        s if !s.is_empty() => match std::str::from_utf8(s) {
            Ok(name) => name,
            Err(_) => {
                println!("Error: Failed to interpret C string as UTF-8.");
                return 0;
            }
        },
        _ => {
            println!("Error: Received an empty process name.");
            return 0;
        }
    };

    // Attach to the process
    let mut process_list = PROCESS_LIST.lock().unwrap();
    match Process::with_name(process_name, &mut process_list) {
        Ok(process) => {
            println!(
                "Successfully attached to process: {} (PID: {})",
                process
                    .name
                    .clone()
                    .unwrap_or("<Unnamed Process>".to_string()),
                process.pid
            );
            process.pid
        }
        Err(e) => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn process_detach(process: c_ulong) -> c_int {
    let mut processes = PROCESS_LIST.lock().unwrap();

    // Convert process handle to i32 (i32) to match the key type
    let pid = process as i32;

    if processes.remove(pid).is_some() {
        println!("Detached from process with handle: {}", process);
        0 // Success
    } else {
        println!("Error: Invalid process handle {}", process);
        -1 // Error
    }
}

#[no_mangle]
pub unsafe extern "C" fn process_get_module_address(
    process: c_ulong,
    ptr: *const c_char,
    len: c_uint,
) -> c_ulong {
    if ptr.is_null() {
        println!("Error: Null pointer for module name.");
        return 0;
    }

    // Convert the C string to a Rust string using the given length
    let module_name = match std::slice::from_raw_parts(ptr as *const u8, len as usize) {
        s if !s.is_empty() => match std::str::from_utf8(s) {
            Ok(name) => name,
            Err(_) => {
                println!("Error: Failed to interpret module name as UTF-8.");
                return 0;
            }
        },
        _ => {
            println!("Error: Received an empty module name.");
            return 0;
        }
    };

    let mut processes = PROCESS_LIST.lock().unwrap();
    let pid = process as i32;

    match processes.get_mut(&pid) {
        Some(proc) => {
            if let Some(address) = proc.get_module_address(module_name) {
                address
            } else {
                0
            }
        }
        None => {
            println!("Error: Invalid process handle: {}", process);
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn process_get_module_size(
    process: c_ulong,
    ptr: *const c_char,
    len: c_uint,
) -> c_ulong {
    if ptr.is_null() {
        println!("Error: Null pointer for module name.");
        return 0;
    }

    // Convert the C string to a Rust string using the given length
    let module_name = match std::slice::from_raw_parts(ptr as *const u8, len as usize) {
        s if !s.is_empty() => match std::str::from_utf8(s) {
            Ok(name) => name,
            Err(_) => {
                println!("Error: Failed to interpret module name as UTF-8.");
                return 0;
            }
        },
        _ => {
            println!("Error: Received an empty module name.");
            return 0;
        }
    };

    let mut processes = PROCESS_LIST.lock().unwrap();
    let pid = process as i32;

    match processes.get_mut(&pid) {
        Some(proc) => {
            if let Some(size) = proc.get_module_size(module_name) {
                size
            } else {
                0
            }
        }
        None => {
            println!("Error: Invalid process handle: {}", process);
            0
        }
    }
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn process_read(
    process: c_ulong,
    address: c_ulong,
    buf_ptr: *mut c_char,
    buf_len: c_uint,
) -> c_uint {
    if buf_ptr.is_null() || buf_len == 0 {
        println!("Error: Null pointer or zero-length buffer.");
        return 0;
    }

    // Access the global process list
    let mut processes = PROCESS_LIST.lock().unwrap();
    let pid = process as i32;

    // Find the process
    match processes.get_mut(&pid) {
        Some(proc) => {
            // Prepare the buffer for reading
            let local_iov = iovec {
                iov_base: buf_ptr as *mut _,
                iov_len: buf_len as usize,
            };
            let remote_iov = iovec {
                iov_base: address as *mut _,
                iov_len: buf_len as usize,
            };

            // Use process_vm_readv to read memory from the target process
            let bytes_read = process_vm_readv(
                pid,
                &local_iov as *const iovec,
                1,
                &remote_iov as *const iovec,
                1,
                0,
            );

            if bytes_read == -1 {
                // let err = std::io::Error::last_os_error();
                // println!(
                //     "Failed to read memory from process {} at address {}: {}",
                //     pid, address, err
                // );
                return 0;
            }
            1
        }
        None => {
            println!("Error: Invalid process handle: {}", process);
            0
        }
    }
}
#[cfg(target_os = "windows")]
#[no_mangle]
pub unsafe extern "C" fn process_read(
    process: c_ulong,
    address: c_ulong,
    buf_ptr: *mut c_char,
    buf_len: c_uint,
) -> c_uint {
    if buf_ptr.is_null() || buf_len == 0 {
        println!("Error: Null pointer or zero-length buffer.");
        return 0;
    }

    let mut processes = PROCESS_LIST.lock().unwrap();
    let pid = process as i32;

    match processes.get_mut(&pid) {
        Some(proc) => {
            // Open the process using the PID we stored earlier
            let handle = match OpenProcess(
                PROCESS_VM_READ | PROCESS_QUERY_INFORMATION,
                false,
                proc.pid as u32,
            ) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("Failed to open process {}: {:?}", proc.pid, e);
                    return 0;
                }
            };

            if handle.is_invalid() {
                eprintln!("Invalid handle returned for process {}", proc.pid);
                return 0;
            }

            let mut bytes_read: usize = 0;
            let success = ReadProcessMemory(
                handle,
                address as *const _,
                buf_ptr as *mut _,
                buf_len as usize,
                Some(&mut bytes_read),
            );

            let _ = CloseHandle(handle);

            if success.is_ok() {
                1
            } else {
                // eprintln!(
                //     "Failed to read memory from process {} at address {:#x}",
                //     proc.pid, address
                // );
                0
            }
        }
        None => {
            println!("Error: Invalid process handle: {}", process);
            0
        }
    }
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub unsafe extern "C" fn process_list_by_name(ptr: *const c_char, len: c_uint) -> c_uint {
    if ptr.is_null() || len == 0 {
        eprintln!("process_list_by_name: null or empty string");
        return 0;
    }

    // Convert to Rust string
    let name = match std::slice::from_raw_parts(ptr as *const u8, len as usize) {
        s if !s.is_empty() => match std::str::from_utf8(s) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("process_list_by_name: invalid UTF-8");
                return 0;
            }
        },
        _ => {
            eprintln!("process_list_by_name: empty name");
            return 0;
        }
    };

    let mut list = PROCESS_LIST.lock().unwrap();
    list.refresh(); // Make sure it's current

    let matches: Vec<_> = list.processes_by_name(name).collect();
    println!(
        "process_list_by_name: found {} processes matching '{}'",
        matches.len(),
        name
    );

    matches.len() as c_uint
}

#[derive(Debug)]
struct Process {
    pid: i32,
    name: Option<String>,
    start_time: Instant,
}

#[cfg(target_os = "linux")]
impl Process {
    fn with_name(name: &str, process_list: &mut ProcessList) -> Result<Self, String> {
        process_list.refresh();
        let processes = process_list.processes_by_name(name);

        let process = processes
            .max_by_key(|p| (p.start_time, p.pid))
            .ok_or_else(|| format!("No matching process found for '{}'", name))?;

        let path = process
            .name
            .clone()
            .unwrap_or("<Unnamed Process>".to_string());
        let pid = process.pid;

        let now = std::time::Instant::now();
        Ok(Process {
            pid,
            name: Some(path),
            start_time: now,
        })
    }

    fn get_module_address(&self, module_name: &str) -> Option<c_ulong> {
        let maps_path = format!("/proc/{}/maps", self.pid);
        if let Ok(contents) = std::fs::read_to_string(&maps_path) {
            for line in contents.lines() {
                if line.contains(module_name) {
                    // Example line format: "7f5a6be00000-7f5a6c000000 r-xp 00000000 fd:01 131076 /lib/x86_64-linux-gnu/libc-2.31.so"
                    if let Some(address_range) = line.split_whitespace().next() {
                        if let Some(start_address) = address_range.split('-').next() {
                            if let Ok(address) = u64::from_str_radix(start_address, 16) {
                                return Some(address as c_ulong);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_module_size(&self, module_name: &str) -> Option<c_ulong> {
        let maps_path = format!("/proc/{}/maps", self.pid);
        if let Ok(contents) = std::fs::read_to_string(&maps_path) {
            for line in contents.lines() {
                if line.contains(module_name) {
                    // Example line format: "7f5a6be00000-7f5a6c000000 r-xp 00000000 fd:01 131076 /lib/x86_64-linux-gnu/libc-2.31.so"
                    if let Some(address_range) = line.split_whitespace().next() {
                        let mut parts = address_range.split('-');
                        if let (Some(start), Some(end)) = (parts.next(), parts.next()) {
                            if let (Ok(start_addr), Ok(end_addr)) =
                                (u64::from_str_radix(start, 16), u64::from_str_radix(end, 16))
                            {
                                let size = end_addr - start_addr;
                                return Some(size as c_ulong);
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(target_os = "windows")]
impl Process {
    fn with_name(name: &str, process_list: &mut ProcessList) -> Result<Self, String> {
        process_list.refresh();
        let processes = process_list.processes_by_name(name);

        let process = processes
            .max_by_key(|p| (p.start_time, p.pid))
            .ok_or_else(|| format!("No matching process found for '{}'", name))?;

        Ok(Process {
            pid: process.pid,
            name: process.name.clone(),
            start_time: std::time::Instant::now(),
        })
    }

    fn get_module_address(&self, module_name: &str) -> Option<c_ulong> {
        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid as u32)
                .ok()?
        };

        let mut module_entry = MODULEENTRY32W::default();
        module_entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;

        unsafe {
            if Module32FirstW(snapshot, &mut module_entry).is_ok() {
                loop {
                    let name_u16 = &module_entry.szModule;
                    let null_pos = name_u16
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(name_u16.len());
                    let name = String::from_utf16_lossy(&name_u16[..null_pos]);

                    // Loose match: substring ignore-case
                    if name
                        .to_ascii_lowercase()
                        .contains(&module_name.to_ascii_lowercase())
                    {
                        return Some(module_entry.modBaseAddr as usize as c_ulong);
                    }

                    if !Module32NextW(snapshot, &mut module_entry).is_ok() {
                        break;
                    }
                }
            }
        }

        println!("Module not found: {}", module_name);
        None
    }

    fn get_module_size(&self, module_name: &str) -> Option<c_ulong> {
        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid as u32)
                .ok()?
        };

        let mut module_entry = MODULEENTRY32W::default();
        module_entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;

        unsafe {
            if Module32FirstW(snapshot, &mut module_entry).is_ok() {
                loop {
                    let name_u16 = &module_entry.szModule;
                    let null_pos = name_u16
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(name_u16.len());
                    let name = String::from_utf16_lossy(&name_u16[..null_pos]);

                    println!("Checking module size for: {}", name);

                    // Loose substring match
                    if name
                        .to_ascii_lowercase()
                        .contains(&module_name.to_ascii_lowercase())
                    {
                        return Some(module_entry.modBaseSize as c_ulong);
                    }

                    if !Module32NextW(snapshot, &mut module_entry).is_ok() {
                        break;
                    }
                }
            }
        }

        println!("Module size not found: {}", module_name);
        None
    }
}

struct ProcessList {
    processes: HashMap<i32, Process>,
}

#[cfg(target_os = "linux")]
impl ProcessList {
    fn new() -> Self {
        ProcessList {
            processes: HashMap::new(),
        }
    }

    fn insert(&mut self, pid: i32, process: Process) {
        self.processes.insert(pid, process);
    }

    fn remove(&mut self, pid: i32) -> Option<Process> {
        self.processes.remove(&pid)
    }

    fn refresh(&mut self) {
        self.processes.clear();

        let proc_dir = match std::fs::read_dir("/proc") {
            Ok(dir) => dir,
            Err(_) => {
                println!("Error: Unable to read /proc directory.");
                return;
            }
        };

        for entry in proc_dir {
            if let Ok(entry) = entry {
                if let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() {
                    let cmdline_path = format!("/proc/{}/cmdline", pid);
                    if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                        // The command line is null-separated; take only the first segment.
                        let name = cmdline.split('\0').next().unwrap_or("").to_string();

                        // Use the executable name itself, not the full path
                        let executable_name = name.rsplit('/').next().unwrap_or("").to_string();

                        if !executable_name.is_empty() {
                            self.processes.insert(
                                pid,
                                Process {
                                    pid,
                                    name: Some(executable_name),
                                    start_time: std::time::Instant::now(),
                                },
                            );
                        }
                    }
                }
            }
        }

        if self.processes.is_empty() {
            println!("No processes found in /proc.");
        }
    }

    fn get_mut(&mut self, pid: &i32) -> Option<&mut Process> {
        self.processes.get_mut(pid)
    }

    fn processes_by_name<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &Process> + 'a {
        let name_bytes = name.as_bytes();

        self.processes.values().filter(move |p| {
            if let Some(ref pname) = p.name {
                pname.as_bytes() == name_bytes
            } else {
                false
            }
        })
    }
}

#[cfg(target_os = "windows")]
impl ProcessList {
    fn new() -> Self {
        ProcessList {
            processes: HashMap::new(),
        }
    }

    fn insert(&mut self, pid: i32, process: Process) {
        self.processes.insert(pid, process);
    }

    fn remove(&mut self, pid: i32) -> Option<Process> {
        self.processes.remove(&pid)
    }

    fn refresh(&mut self) {
        self.processes.clear();

        unsafe {
            let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(snap) => snap,
                Err(e) => {
                    eprintln!("CreateToolhelp32Snapshot failed: {:?}", e);
                    return;
                }
            };

            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let exe_name = String::from_utf16_lossy(
                        &entry.szExeFile
                            [..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)],
                    );

                    if !exe_name.is_empty() {
                        self.processes.insert(
                            entry.th32ProcessID as i32,
                            Process {
                                pid: entry.th32ProcessID as i32,
                                name: Some(exe_name),
                                start_time: Instant::now(),
                            },
                        );
                    }

                    if !Process32NextW(snapshot, &mut entry).is_ok() {
                        break;
                    }
                }
            }
        }
    }

    fn get_mut(&mut self, pid: &i32) -> Option<&mut Process> {
        self.processes.get_mut(pid)
    }

    fn processes_by_name<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &Process> + 'a {
        let name_bytes = name.as_bytes();

        self.processes.values().filter(move |p| {
            if let Some(ref pname) = p.name {
                pname.as_bytes() == name_bytes
            } else {
                false
            }
        })
    }
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn user_settings_add_file_select(_key: *const c_char, _desc: *const c_char) {
    println!("user_settings_add_file_select: stub called");
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn user_settings_add_file_select_name_filter(
    _key: *const c_char,
    _filter: *const c_char,
) {
    println!("user_settings_add_file_select_name_filter: stub called");
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn user_settings_add_file_select_mime_filter(
    _key: *const c_char,
    _mime: *const c_char,
) {
    println!("user_settings_add_file_select_mime_filter: stub called");
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_map_load() -> *mut std::ffi::c_void {
    println!("settings_map_load: stub called");
    std::ptr::null_mut()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_map_get(
    _map: *mut std::ffi::c_void,
    _key: *const c_char,
) -> *mut std::ffi::c_void {
    println!("settings_map_get: stub called");
    std::ptr::null_mut()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_map_free(_map: *mut std::ffi::c_void) {
    println!("settings_map_free: stub called");
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_map_len(_map: *const core::ffi::c_void) -> u32 {
    println!("settings_map_len: stub called");
    0
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_string(_value: *const core::ffi::c_void) -> *const i8 {
    println!("setting_value_get_string: stub called");
    std::ptr::null()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_map_get_key_by_index(
    _map: *const core::ffi::c_void,
    _index: u32,
) -> *const i8 {
    println!("settings_map_get_key_by_index: stub called");
    std::ptr::null()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_map_get_value_by_index(
    _map: *const core::ffi::c_void,
    _index: u32,
) -> *const core::ffi::c_void {
    println!("settings_map_get_value_by_index: stub called");
    std::ptr::null()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_type(_value: *const core::ffi::c_void) -> u32 {
    println!("setting_value_get_type: stub called");
    0
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_map(
    _value: *const core::ffi::c_void,
) -> *const core::ffi::c_void {
    println!("setting_value_get_map: stub called");
    std::ptr::null()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_list(
    _value: *const core::ffi::c_void,
) -> *const core::ffi::c_void {
    println!("setting_value_get_list: stub called");
    std::ptr::null()
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_bool(_value: *const core::ffi::c_void) -> bool {
    println!("setting_value_get_bool: stub called");
    false
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_i64(_value: *const core::ffi::c_void) -> i64 {
    println!("setting_value_get_i64: stub called");
    0
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_get_f64(_value: *const core::ffi::c_void) -> f64 {
    println!("setting_value_get_f64: stub called");
    0.0
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn setting_value_free(_value: *mut core::ffi::c_void) {
    println!("setting_value_free: stub called");
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_list_free(_list: *mut core::ffi::c_void) {
    println!("settings_list_free: stub called");
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_list_len(_list: *const core::ffi::c_void) -> u32 {
    println!("settings_list_len: stub called");
    0
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn settings_list_get(
    _list: *const core::ffi::c_void,
    _index: u32,
) -> *const core::ffi::c_void {
    println!("settings_list_get: stub called");
    std::ptr::null()
}
