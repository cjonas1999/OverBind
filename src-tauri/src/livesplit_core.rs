use lazy_static::lazy_static;
use libc::{c_char, c_int, c_uint, c_ulong, iovec, pid_t, process_vm_readv};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

lazy_static! {
    static ref PROCESS_LIST: Mutex<ProcessList> = Mutex::new(ProcessList::new());
}

#[no_mangle]
pub unsafe extern "C" fn process_attach(ptr: *const c_char, len: c_uint) -> pid_t {
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

    // Convert process handle to pid_t (i32) to match the key type
    let pid = process as pid_t;

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
    let pid = process as pid_t;

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
    let pid = process as pid_t;

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
    let pid = process as pid_t;

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

#[derive(Debug)]
struct Process {
    pid: pid_t,
    name: Option<String>,
    start_time: Instant,
}

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
                                return Some(address);
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
                                return Some(size);
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

struct ProcessList {
    processes: HashMap<pid_t, Process>,
}

impl ProcessList {
    fn new() -> Self {
        ProcessList {
            processes: HashMap::new(),
        }
    }

    fn insert(&mut self, pid: pid_t, process: Process) {
        self.processes.insert(pid, process);
    }

    fn remove(&mut self, pid: pid_t) -> Option<Process> {
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
                if let Ok(pid) = entry.file_name().to_string_lossy().parse::<pid_t>() {
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

    fn get_mut(&mut self, pid: &pid_t) -> Option<&mut Process> {
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
