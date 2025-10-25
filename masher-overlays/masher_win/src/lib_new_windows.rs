// This file successfully calls `my_present` using the current version of the windows crate. Unfortunately the dx11 renderer for imgui does NOT support current windows so I had to
// scrap this and use a totally different approach that was compatible with the old windows crate. But I ended up forking the dx11 renderer anyway. So there's a good chance I can go
// back to this approach later which is why I'm checking it into VC. But since the old version technically works I'm just leaving this as a possible TODO

#![allow(non_snake_case)]
use minhook_sys::{MH_CreateHook, MH_EnableHook, MH_Initialize};
use std::ffi::{c_void, CString};
use std::ptr::{self, null_mut, NonNull};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::{Once, OnceLock};
use std::{mem, panic};
use std::mem::ManuallyDrop;
use widestring::U16CString;
use windows::core::{Error, Result as WinResult, BOOL, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    E_FAIL, FARPROC, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED, DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::DXGI_SWAP_EFFECT_DISCARD;
use windows::Win32::Graphics::Dxgi::{
    Common::DXGI_FORMAT_R8G8B8A8_UNORM, IDXGISwapChain,
    DXGI_SWAP_CHAIN_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, GetProcAddress, LoadLibraryExA, LoadLibraryW,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Interface;
use imgui::Context as ImGuiContext;

#[repr(C)]
pub struct DXGI_SWAP_CHAIN_VTBL {
    // We'll only access Present (index 8). The vtable has many entries;
    // we will treat it as pointer array.
    _reserved: [usize; 20],
}

#[repr(transparent)]
struct RendererDevicePlaceholder(*mut std::ffi::c_void);

// function pointer signatures
type PresentFn =
    unsafe extern "system" fn(this: *mut c_void, SyncInterval: u32, Flags: u32) -> HRESULT;

// Global place to store original Present pointer
static ORIG_PRESENT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
// Flag to initialized overlay once
static INIT_IMGUI: Once = Once::new();
static mut GLOBAL_CTX: Option<NonNull<ImGuiContext>> = None;
static IMGUI_READY: AtomicBool = AtomicBool::new(false);
static START_ONCE: Once = Once::new();
static REAL_D3D11: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, w, l)
}

pub unsafe fn create_hidden_window() -> WinResult<HWND> {
    let class_name = widestring::U16CString::from_str("DummyWndClass").unwrap();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(null_mut()),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };

    RegisterClassW(&wc);

    // Call CreateWindowExW and unwrap the Result
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class_name.as_ptr()),
        PCWSTR(class_name.as_ptr()),
        WS_OVERLAPPEDWINDOW,
        0,
        0,
        1,
        1,
        Some(HWND(null_mut())),      // parent
        None,         // menu
        Some(HINSTANCE(null_mut())), // instance
        None,         // param
    )?;

    Ok(hwnd)
}

pub unsafe fn create_dummy_d3d11_swapchain() -> WinResult<(
    HWND,
    Option<ID3D11Device>,
    Option<ID3D11DeviceContext>,
    Option<IDXGISwapChain>,
)> {
    let hwnd = create_hidden_window()?;

    let mode_desc = DXGI_MODE_DESC {
        Width: 1,
        Height: 1,
        RefreshRate: Default::default(),
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
        Scaling: DXGI_MODE_SCALING_UNSPECIFIED,
    };

    let sample_desc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };

    let swap_desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: mode_desc,
        SampleDesc: sample_desc,
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 1,
        OutputWindow: hwnd,
        Windowed: BOOL(1), // true
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        Flags: 0,
    };

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut swapchain: Option<IDXGISwapChain> = None;

    let hr = D3D11CreateDeviceAndSwapChain(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE(null_mut()),
        D3D11_CREATE_DEVICE_FLAG(0),
        None,
        D3D11_SDK_VERSION,
        Some(&swap_desc),
        Some(&mut swapchain),
        Some(&mut device),
        None,
        Some(&mut context),
    );

    if hr.is_err() {
        dbg(&format!(
            "Dummy device creation failed: {}",
            hr.as_ref().unwrap_err()
        ));
        return Err(hr.unwrap_err());
    }

    dbg("Dummy swapchain created");
    Ok((hwnd, device, context, swapchain))
}

// Utility: load real d3d11 from System32
fn load_real_d3d11() -> WinResult<HMODULE> {
    // Build "C:\Windows\System32\d3d11.dll"
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) }; // returns length (u32)
    if n == 0 || (n as usize) >= buf.len() {
        dbg("GetSystemDirectoryW failed");
        return Err(Error::new(E_FAIL, "GetSystemDirectoryW failed"));
    }
    let mut path = String::from_utf16_lossy(&buf[..n as usize]);
    if !path.ends_with('\\') {
        path.push('\\');
    }
    path.push_str("d3d11.dll");

    let w = widestring::U16CString::from_str(&path).unwrap();
    dbg(&format!("Loading real d3d11 by path: {path}"));

    // Load by absolute path to avoid our own proxy
    let h = unsafe { LoadLibraryW(PCWSTR(w.as_ptr())) }?;
    Ok(h)
}

fn get_real_d3d11() -> WinResult<HMODULE> {
    // Fast path: already cached
    let cached = REAL_D3D11.load(Ordering::SeqCst);
    if !cached.is_null() {
        return Ok(HMODULE(cached));
    }

    // Load and cache atomically
    let h = load_real_d3d11()?;
    REAL_D3D11.store(h.0, Ordering::SeqCst);
    Ok(h)
}

// A very small stub for an overlay renderer. Replace with your ImGui render code.
unsafe fn render_overlay_stub(device: *mut c_void, context: *mut c_void, swap: *mut c_void) {
    // TODO: initialize ImGui with device/context once; upload fonts; draw frame.
    // For demo, we do nothing or maybe a MessageBox (not recommended in real hooks)
    // MessageBoxA(0, CString::new("Overlay frame").unwrap().as_ptr(), CString::new("Info").unwrap().as_ptr(), 0);
    dbg("Calling render_overlay_stub");
}

pub unsafe fn minhook_init_and_hook(target: *mut c_void, detour: *mut c_void) {
    dbg(&format!("minhook_init_and_hook: target={target:p}, detour={detour:p}"));

    if MH_Initialize() != 0 { dbg("MH_Initialize failed"); return; }
    dbg("MH_Initialize ok");

    let mut orig: *mut c_void = std::ptr::null_mut();
    let r1 = MH_CreateHook(target, detour, &mut orig as *mut _ as *mut _);
    dbg(&format!("MH_CreateHook: {r1}"));
    if r1 == 0 {
        ORIG_PRESENT.store(orig, Ordering::SeqCst);
    }
    let r2 = MH_EnableHook(target);
    dbg(&format!("MH_EnableHook: {r2}"));
}


pub unsafe fn get_present_addr_from_swapchain(sc: &IDXGISwapChain) -> *mut core::ffi::c_void {
    // sc.as_raw() → *mut IDXGISwapChain (COM interface)
    let raw = sc.as_raw() as *mut IDXGISwapChain;

    if raw.is_null() {
        dbg("get_present_addr_from_swapchain: sc.as_raw() is null");
        return std::ptr::null_mut();
    }

    // COM layout: first field is vtable pointer (**this)
    let vtbl_ptr_ptr = raw as *mut *mut *mut core::ffi::c_void;
    if vtbl_ptr_ptr.is_null() {
        dbg("get_present_addr_from_swapchain: vtbl_ptr_ptr is null");
        return std::ptr::null_mut();
    }

    let vtbl = *vtbl_ptr_ptr; // *mut *mut c_void (array of function pointers)
    if vtbl.is_null() {
        dbg("get_present_addr_from_swapchain: vtbl is null");
        return std::ptr::null_mut();
    }

    // IDXGISwapChain::Present is vtable slot 8 (0-based)
    let present = *vtbl.add(8);
    dbg(&format!("get_present_addr_from_swapchain: Present = {present:p}"));
    present
}


fn init_present_hook() {
    unsafe {
        match create_dummy_d3d11_swapchain() {
            Ok((_hwnd, device, context, swapchain)) => {
                if let Some(sc) = &swapchain {
                    let present_addr = get_present_addr_from_swapchain(sc);
                    minhook_init_and_hook(present_addr, my_present as *mut _);
                    dbg("Present detour installed");
                } else {
                    dbg("Dummy swapchain was None");
                }
            }
            Err(e) => {
                dbg(&format!("Dummy swapchain creation failed: {e:?}"));
            }
        }
    }
}

// Our Present hook (matching IDXGISwapChain::Present signature)

extern "system" fn my_present(this: *mut core::ffi::c_void, sync: u32, flags: u32) -> HRESULT {
    dbg("my_present called");
    // unsafe {
    //     INIT_IMGUI.call_once(|| {
    //         dbg("my_present: first call – initializing ImGui");

    //         // 1. Get the swapchain and device
    //         let swap = this as *mut IDXGISwapChain;
    //         let dev = match (*swap).GetDevice::<ID3D11Device>() {
    //             Ok(d) => d,
    //             Err(e) => {
    //                 dbg(&format!("GetDevice failed: {e:?}"));
    //                 return;
    //             }
    //         };

    //         // 2. Create ImGui context
    //         let mut ctx = imgui::Context::create();
    //         ctx.set_ini_filename(None);
    //         dbg("Created global ImGui context");

    //         let dev: ID3D11Device = (*swap).GetDevice().unwrap();
    //         let mut renderer = imgui_dx11_renderer::Renderer::new(&mut ctx, &dev)?;

                
    //         // 4. Store everything globally
    //         GLOBAL_CTX = Some(NonNull::new_unchecked(Box::leak(Box::new(ctx))));
    //         IMGUI_READY.store(true, Ordering::SeqCst);
    //         dbg("ImGui context + renderer initialized");
    //     });

    //     if IMGUI_READY.load(Ordering::SeqCst) {
    //         let ctx = GLOBAL_CTX.unwrap().as_mut();
    //         // ... per-frame drawing:
    //         let ui = ctx.frame();

    //         {
    //             let draw_list = ui.get_foreground_draw_list();
    //             draw_list.add_text([30.0, 30.0], imgui::ImColor32::from_rgba(255, 0, 0, 255), "Hello from ImGui!");
    //         }

    //         // Flush via your renderer
    //         // If you stored the renderer in another global, call it here.
    //         // Example:
    //         // if let Some(renderer) = GLOBAL_RENDERER.as_mut() { renderer.render(ctx); }
    //     }
    // }

    // Always call the original Present
    unsafe {
        let orig_ptr = ORIG_PRESENT.load(Ordering::SeqCst);
        let orig: extern "system" fn(*mut core::ffi::c_void, u32, u32) -> HRESULT =
        std::mem::transmute(orig_ptr);
        orig(this, sync, flags)
    }
}


// Replace the Present pointer in the swapchain vtable with our hook
unsafe fn hook_swapchain_present(
    swapchain_raw: *mut core::ffi::c_void,
) -> Result<(), &'static str> {
    if swapchain_raw.is_null() {
        dbg("hook_swapchain_present: swapchain null");
        return Err("swapchain null");
    }

    // A COM object pointer layout: first field is pointer to vtable
    let vtable_ptr_ptr = swapchain_raw as *mut *mut *mut core::ffi::c_void;
    if vtable_ptr_ptr.is_null() {
        dbg("hook_swapchain_present: vtable_ptr_ptr null");
        return Err("vtable_ptr_ptr null");
    }
    let vtable = *vtable_ptr_ptr; // *mut *mut c_void
    if vtable.is_null() {
        dbg("hook_swapchain_present: vtable null");
        return Err("vtable null");
    }

    // Present is usually vtable index 8
    const PRESENT_INDEX: isize = 8;
    let present_slot = vtable.offset(PRESENT_INDEX);

    // Read original function pointer
    let orig_present_ptr = *present_slot as *mut core::ffi::c_void;
    ORIG_PRESENT.store(orig_present_ptr, Ordering::SeqCst);

    // Change memory protection to writable. VirtualProtect expects a pointer to PAGE_PROTECTION_FLAGS
    let mut old_protect = PAGE_PROTECTION_FLAGS(0);
    let size = mem::size_of::<*mut core::ffi::c_void>();

    // VirtualProtect returns BOOL wrapped in windows crate -> use .as_bool() or check != 0
    VirtualProtect(
        present_slot as *mut core::ffi::c_void,
        size,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    )
    .map_err(|_| "VirtualProtect (set RW) failed")?; // propagate the error if it fails

    // Write our Present pointer
    // NOTE: cast to pointer types carefully
    let mine = my_present as *const () as *mut core::ffi::c_void;
    *present_slot = mine;

    // Restore old protection
    VirtualProtect(
        present_slot as *mut core::ffi::c_void,
        size,
        old_protect,
        &mut old_protect,
    )
    .map_err(|_| "VirtualProtect (restore) failed")?;

    Ok(())
}

extern "system" fn init_thread(_param: *mut c_void) -> u32 {
    install_panic_hook(); // your dbg()-backed panic hook
    dbg("init thread: starting");
    // do your real init here (see section 2 below)
    init_present_hook(); // <— implement below
    dbg("init thread: done");
    0
}

#[no_mangle]
pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        dbg("DllMain: DLL_PROCESS_ATTACH");
        unsafe { let _ = DisableThreadLibraryCalls(hinst.into()); }

        std::thread::spawn(|| {
            dbg("init_thread: started");
            install_panic_hook();
            dbg("init_thread: panic hook installed");

            match unsafe { create_dummy_d3d11_swapchain() } {
                Ok((_hwnd, _dev, _ctx, sc)) => {
                    dbg("init_thread: dummy swapchain created");
                    if let Some(sc) = &sc {
                        let present_addr = unsafe { get_present_addr_from_swapchain(sc) };
                        if present_addr.is_null() {
                            dbg("init_present_hook: Present addr is NULL – aborting hook");
                            return;
                        }
                        unsafe { minhook_init_and_hook(present_addr, my_present as *mut _) };
                        dbg("init_thread: hook installed");
                    }
                }
                Err(e) => dbg(&format!("init_thread: dummy swapchain failed: {e:?}")),
            }

            dbg("init_thread: finished");
        });
    }
    BOOL(1)
}


fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let mut message = String::from("PANIC: ");
        if let Some(s) = info.payload().downcast_ref::<&str>() {
            message.push_str(s);
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            message.push_str(s);
        } else {
            message.push_str("unknown panic");
        }
        if let Some(loc) = info.location() {
            use std::fmt::Write;
            let _ = write!(&mut message, " ({}:{})", loc.file(), loc.line());
        }
        dbg(&message);
    }));
}

fn dbg(msg: &str) {
    if let Ok(w) = U16CString::from_str(msg) {
        unsafe { OutputDebugStringW(PCWSTR(w.as_ptr())) }
    }
}
