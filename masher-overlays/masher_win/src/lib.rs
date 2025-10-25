// Cargo.toml must set crate-type = ["cdylib"]
// target: i686-pc-windows-msvc (32-bit)

#![allow(non_snake_case)]
use minhook_sys::{MH_CreateHook, MH_EnableHook, MH_Initialize};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use std::ffi::{c_void, CString, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{self, null, null_mut, NonNull};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicU32, Ordering};
use std::sync::{Once, OnceLock};
use std::time::Duration;
use std::{mem, panic, thread};
use std::mem::{transmute_copy, ManuallyDrop};
use widestring::U16CString;
use windows::core::{Error, Result as WinResult, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, BOOL, ERROR_PIPE_CONNECTED, E_FAIL, FARPROC, HINSTANCE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceChild, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_VIEWPORT
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED, DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{IDXGIFactory, IDXGIFactory2, IDXGIOutput, IDXGISwapChain1, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FULLSCREEN_DESC, DXGI_SWAP_EFFECT_DISCARD};
use windows::Win32::Graphics::Dxgi::{
    Common::DXGI_FORMAT_R8G8B8A8_UNORM, IDXGISwapChain,
    DXGI_SWAP_CHAIN_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, GetModuleHandleA, GetProcAddress, LoadLibraryExA, LoadLibraryW, LOAD_LIBRARY_SEARCH_SYSTEM32
};
use windows::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Interface;
use imgui::{Condition, Context as ImGuiContext};
use imgui_dx11_renderer::Renderer;

type HMODULE = isize;

#[repr(C)]
pub struct DXGI_SWAP_CHAIN_VTBL {
    // We'll only access Present (index 8). The vtable has many entries;
    // we will treat it as pointer array.
    _reserved: [usize; 20],
}

// Global place to store original Present pointer
static ORIG_PRESENT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
// Flag to initialized overlay once
static INIT_IMGUI: Once = Once::new();
static mut GLOBAL_CTX: Option<ImGuiContext> = None;
static mut GLOBAL_RENDERER: Option<Renderer> = None;
static mut GLOBAL_SWAPCHAIN: Option<ManuallyDrop<IDXGISwapChain>> = None;
static mut GLOBAL_DEVICE: Option<ManuallyDrop<ID3D11Device>> = None;
static mut GLOBAL_DEVICE_CTX: Option<ManuallyDrop<ID3D11DeviceContext>> = None;
static mut RAW_SWAPCHAIN: *mut c_void = null_mut();
static mut RAW_DEVICE: *mut c_void = null_mut();
static mut RAW_DEVICE_CTX: *mut c_void = null_mut();
static mut GLOBAL_BACKBUFFER_RTV: Option<ID3D11RenderTargetView> = None;
static mut GLOBAL_FB_SIZE: (u32, u32) = (0, 0);
static IMGUI_READY: AtomicBool = AtomicBool::new(false);
static START_ONCE: Once = Once::new();
static REAL_D3D11: AtomicIsize = AtomicIsize::new(0);
static MH_INIT_DONE: AtomicBool = AtomicBool::new(false);
static HOOKS_INSTALLED_FOR_D3D: AtomicBool = AtomicBool::new(false);
static PRESENT_HOOKED: AtomicBool = AtomicBool::new(false);

unsafe fn borrow_device() -> Option<&'static windows::Win32::Graphics::Direct3D11::ID3D11Device> {
    if RAW_DEVICE.is_null() { return None; }
    Some(&*(RAW_DEVICE as *mut windows::Win32::Graphics::Direct3D11::ID3D11Device))
}

unsafe fn wrap_device_from_raw(raw: *mut std::ffi::c_void) -> ManuallyDrop<ID3D11Device> {
    let dev: ID3D11Device = transmute_copy(&raw);
    ManuallyDrop::new(dev) // prevent Drop => no Release
}

unsafe fn wrap_device_context_from_raw(raw: *mut std::ffi::c_void) -> ManuallyDrop<ID3D11DeviceContext> {
    let ctx: ID3D11DeviceContext = transmute_copy(&raw);
    ManuallyDrop::new(ctx) // prevent Drop => no Release
}

unsafe fn wrap_swapchain_from_raw(raw: *mut std::ffi::c_void) -> ManuallyDrop<IDXGISwapChain> {
    let sc: IDXGISwapChain = transmute_copy(&raw);
    ManuallyDrop::new(sc) // prevent Drop => no Release
}

unsafe fn borrow_context() -> Option<&'static windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext> {
    if RAW_DEVICE_CTX.is_null() { return None; }
    Some(&*(RAW_DEVICE_CTX as *mut windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext))
}

unsafe fn borrow_swapchain() -> Option<&'static windows::Win32::Graphics::Dxgi::IDXGISwapChain> {
    if RAW_SWAPCHAIN.is_null() { return None; }
    Some(&*(RAW_SWAPCHAIN as *mut windows::Win32::Graphics::Dxgi::IDXGISwapChain))
}

unsafe fn ensure_rtv_from_sc() -> windows::core::Result<()> {
    let sc = &*GLOBAL_SWAPCHAIN.as_ref().unwrap();
    // 1) Grab backbuffer
    dbg("About to call GetBuffer");
    let backbuffer: ID3D11Texture2D = match sc.GetBuffer(0) {
        Ok(tex) => tex,
        Err(e) => {
            // If the engine is mid-resize, just skip this frame without crashing
            dbg(&format!("GetBuffer(0) failed: {e:?}"));
            return Ok(());
        }
    };
    dbg("Grabbed backbuffer");

    // 2) Read its size from the texture (avoid SwapChain::GetDesc)
    let (w, h) = texture_size(&backbuffer)?;
    dbg("Read texture size");

    // 3) (Re)create RTV if needed
    if GLOBAL_BACKBUFFER_RTV.is_none() || GLOBAL_FB_SIZE != (w, h) {
        recreate_rtv_from_tex(&backbuffer)?;
        GLOBAL_FB_SIZE = (w, h);
    }
    dbg("recreated rtv from tex");

    Ok(())
}

unsafe fn texture_size(tex: &ID3D11Texture2D) -> windows::core::Result<(u32, u32)> {
    // For `windows = 0.36`, Texture2D::GetDesc takes an OUT param (older style) *or*
    // returns the struct, depending on feature level. Use a defensive approach:
    #[allow(unused_mut)]
    let mut desc: D3D11_TEXTURE2D_DESC = std::mem::zeroed();

    // Try the old-style OUT param call (works across 0.36):
    // If your method set returns the struct directly, just replace the next two lines with:
    //   let desc: D3D11_TEXTURE2D_DESC = tex.GetDesc();
    tex.GetDesc(&mut desc);
    Ok((desc.Width, desc.Height))
}

unsafe fn recreate_rtv_from_tex(backbuffer: &ID3D11Texture2D) -> windows::core::Result<()> {
    GLOBAL_BACKBUFFER_RTV = None;

    if let Some(dev) = GLOBAL_DEVICE.as_ref() {
        // CreateRenderTargetView expects a raw desc pointer; pass null for default
        let rtv = dev.CreateRenderTargetView(backbuffer, std::ptr::null())?;
        GLOBAL_BACKBUFFER_RTV = Some(rtv);
    }

    Ok(())
}

unsafe fn ensure_imgui_initialized() -> windows::core::Result<()> {
    if GLOBAL_CTX.is_some() {
        return Ok(());
    }

    INIT_IMGUI.call_once(|| {
        let mut ctx = ImGuiContext::create();
        ctx.io_mut().font_global_scale = 1.0;
        ctx.fonts().add_font(&[imgui::FontSource::DefaultFontData { config: None }]);
        dbg("Created new context with font");
        let device = borrow_device().unwrap();
        dbg("Got device from RAW_DEVICE");
        let renderer = unsafe {
            Renderer::new(&mut ctx, device)
                .expect("Failed to create imgui_dx11_renderer")
        };
        dbg("Created new renderer");

        start_pipe_thread();

        unsafe {
            GLOBAL_CTX = Some(ctx);
            GLOBAL_RENDERER = Some(renderer);
        }
    });

    Ok(())
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
// Our Present hook (matching IDXGISwapChain::Present signature)
extern "system" fn my_present(this: *mut core::ffi::c_void, sync: u32, flags: u32) -> HRESULT {
    unsafe {
        if RAW_DEVICE.is_null() || RAW_DEVICE_CTX.is_null() || RAW_SWAPCHAIN.is_null() {
            return call_original_present(this, sync, flags);
        }
        //dbg("DEVICE, CTX, and SWAPCHAIN all ready");

        let n = FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if n < 200 {
            return call_original_present(this, sync, flags);
        }

        if GLOBAL_SWAPCHAIN.is_none() {
            let raw = this as *mut core::ffi::c_void;
            GLOBAL_SWAPCHAIN = Some(wrap_swapchain_from_raw(raw));
            dbg("[overlay] Wrapped swapchain pointer");
        }

        if !IMGUI_READY.swap(true, Ordering::SeqCst) {
            dbg("[overlay] ImGui Init");
            if let Some(dev_wrap) = GLOBAL_DEVICE.as_ref() {
                let mut imgui = ImGuiContext::create();

                {
                    let mut fonts = imgui.fonts();
                    
                    let font_cfg = imgui::FontConfig {
                        size_pixels: 64.0,
                        ..Default::default()
                    };
                    
                    let font_data = include_bytes!("../../../src-tauri/icons/IconFont.ttf");
                    fonts.add_font(&[imgui::FontSource::TtfData {
                        data: font_data,
                        size_pixels: 64.0,
                        config: Some(font_cfg),
                    }]);
                    fonts.build_rgba32_texture();
                }
                    
                dbg("[overlay] Ready to call Renderer::new");
                match Renderer::new(&mut imgui, &*dev_wrap) {
                    Ok(r) => {
                        GLOBAL_CTX = Some(imgui);
                        GLOBAL_RENDERER = Some(r);
                        dbg("[overlay] ImGui renderer initialized in CreateDevice");
                    }
                    Err(e) => {
                        // If this still fails, we won't try again automatically.
                        IMGUI_READY.store(false, Ordering::SeqCst);
                        dbg(&format!("[overlay] Renderer::new failed in CreateDevice: {:?}", e));
                    }
                }

                start_pipe_thread();
            }
        }
        let mut w = 0f32;
        let mut h: f32 = 0f32;
        if let Some(device_ctx) = GLOBAL_DEVICE_CTX.as_ref() {
            let mut num_viewports: u32 = 1;
            let mut vp: D3D11_VIEWPORT = std::mem::zeroed();
            device_ctx.RSGetViewports(&mut num_viewports as *mut u32, &mut vp as *mut D3D11_VIEWPORT);
            w = vp.Width;
            h = vp.Height;

            // if let Err(e) = ensure_rtv_from_sc() {
            //     dbg(&format!("[overlay] ensure_rtv failed: {:?}", e));
            //     return call_original_present(this, sync, flags);
            // }

            // let rtv = GLOBAL_BACKBUFFER_RTV.as_ref().unwrap();

            // // Bind RTV for ImGui rendering
            // let rtvs = [Some(rtv.clone())];
            // device_ctx.OMSetRenderTargets(&rtvs, None);
        }

        // let (w, h) = GLOBAL_FB_SIZE;
        // if let Some(device_ctx) = GLOBAL_DEVICE_CTX.as_ref() {
        //     let vp = D3D11_VIEWPORT {
        //         TopLeftX: 0.0,
        //         TopLeftY: 0.0,
        //         Width: w as f32,
        //         Height: h as f32,
        //         MinDepth: 0.0,
        //         MaxDepth: 1.0,
        //     };
        //     device_ctx.RSSetViewports(&[vp]);
        // }

        // ImGui render
        {
            let ctx = GLOBAL_CTX.as_mut().unwrap();
            let io = ctx.io_mut();
            io.display_size = [w as f32, h as f32];

            let ui = ctx.frame();
            {
                // Get the foreground draw list (always rendered on top)
                let draw_list = ui.get_foreground_draw_list();

                let visible = MASHER_ACTIVE.load(Ordering::SeqCst);
                //let visible = true;

                let color = if visible {
                    imgui::ImColor32::from_rgba(255, 0, 0, 255) // red
                } else {
                    imgui::ImColor32::from_rgba(100, 100, 100, 150) // dim gray
                };

                // Pick a font (optional — default font will work if it contains the glyph)
                draw_list.add_text([20.0, 60.0], color, " ");
            }

            let draw_data = ui.render();
            GLOBAL_RENDERER
                .as_mut()
                .unwrap()
                .render(draw_data)
                .expect("render failed");
        }
        call_original_present(this, sync, flags)
    }

}

unsafe fn call_original_present(this: *mut c_void, sync: u32, flags: u32) -> HRESULT {
    let orig_ptr = ORIG_PRESENT.load(Ordering::SeqCst);
    let orig: extern "system" fn(*mut core::ffi::c_void, u32, u32) -> HRESULT =
    std::mem::transmute(orig_ptr);
    orig(this, sync, flags)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, w, l)
}

pub unsafe fn create_hidden_window() -> WinResult<HWND> {
    let class_name = widestring::U16CString::from_str("DummyWndClass").unwrap();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(0),
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
        Some(HWND(0)),      // parent
        None,         // menu
        Some(HINSTANCE(0)), // instance
        null_mut(),         // param
    );

    Ok(hwnd)
}

pub unsafe fn create_dummy_d3d11_swapchain() -> WinResult<(*mut IDXGISwapChain, Option<IDXGISwapChain>)> {
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

    let mut swapchain: Option<IDXGISwapChain> = None;
    let device: *mut Option<ID3D11Device> = ptr::null_mut();
    let context: *mut Option<ID3D11DeviceContext> = ptr::null_mut();
    let mut obtained_feature_level = D3D_FEATURE_LEVEL(0i32);
    let feature_levels: [D3D_FEATURE_LEVEL; 0] = [];


    let hr = D3D11CreateDeviceAndSwapChain(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HINSTANCE(0),
        D3D11_CREATE_DEVICE_FLAG(0),
        &feature_levels,
        D3D11_SDK_VERSION,
        &swap_desc,
        ptr::from_mut(&mut swapchain),
        device,
        ptr::from_mut(&mut obtained_feature_level),
        context,
    );

    if hr.is_err() {
        dbg(&format!(
            "Dummy device creation failed: {}",
            hr.as_ref().unwrap_err()
        ));
        return Err(hr.unwrap_err());
    }

    let raw_ptr = match &swapchain {
        Some(sc) => {
            // The Option<T> holds the real COM pointer internally.
            // Reinterpret as pointer-to-pointer and deref it.
            let inner_ptr_ptr = sc as *const IDXGISwapChain as *const *mut core::ffi::c_void;
            *inner_ptr_ptr as *mut IDXGISwapChain
        }
        None => ptr::null_mut(),
    };

    dbg("Dummy swapchain created");
    Ok((raw_ptr, swapchain))
}

// Utility: load real d3d11 from System32
fn load_real_d3d11() -> WinResult<HMODULE> {
    // Build "C:\Windows\System32\d3d11.dll"
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(&mut buf) }; // returns length (u32)
    if n == 0 || (n as usize) >= buf.len() {
        dbg("GetSystemDirectoryW failed");
        return Err(Error::new(E_FAIL, "GetSystemDirectoryW failed".into()));
    }
    let mut path = String::from_utf16_lossy(&buf[..n as usize]);
    if !path.ends_with('\\') {
        path.push('\\');
    }
    path.push_str("d3d11.dll");

    let w = widestring::U16CString::from_str(&path).unwrap();
    dbg(&format!("Loading real d3d11 by path: {path}"));

    // Load by absolute path to avoid our own proxy
    let handle_result = unsafe { LoadLibraryW(PCWSTR(w.as_ptr())) };

    // Handle both cases: `Result<HINSTANCE>` and `HINSTANCE` directly
    let handle: HMODULE = match handle_result {
        Ok(h) => h.0 as HMODULE,
        Err(e) => return Err(e),
    };

    if handle == 0 {
        return Err(windows::core::Error::from_win32());
    }

    Ok(handle)
}

fn get_real_d3d11() -> WinResult<isize> {
    // Fast path: already cached
    let cached = REAL_D3D11.load(Ordering::SeqCst);
    if cached != 0 {
        return Ok(cached);
    }

    // Load and cache atomically
    let h = load_real_d3d11()?;
    REAL_D3D11.store(h, Ordering::SeqCst);
    Ok(h)
}

pub unsafe fn minhook_init_and_hook(target: *mut c_void, detour: *mut c_void) -> Option<*mut c_void> {
    dbg(&format!("minhook_init_and_hook: target={target:p}, detour={detour:p}"));

    let mut orig: *mut c_void = std::ptr::null_mut();
    let r1 = MH_CreateHook(target, detour, &mut orig as *mut _ as *mut _);
    dbg(&format!("MH_CreateHook: {r1}"));
    // if r1 == 0 {
    //     ORIG_PRESENT.store(orig, Ordering::SeqCst);
    // }
    let r2 = MH_EnableHook(target);
    dbg(&format!("MH_EnableHook: {r2}"));

    if r1 == 0 {
        return Some(orig);
    }
    None
}


pub unsafe fn get_present_addr_from_swapchain(sc: *mut IDXGISwapChain) -> *mut core::ffi::c_void {
    // Step 1–2: address-of wrapper, reinterpret as pointer-to-inner-pointer
    if sc.is_null() {
        dbg("sc is null");
        return core::ptr::null_mut();
    }
    dbg("Checked swapchain ptr");

    // Step 4: first field of COM object is vtable pointer
    let vtbl_ptr: *const *const c_void = *(sc as *const *const *const c_void);
    if vtbl_ptr.is_null() {
        dbg("vtbl_ptr is null");
        return core::ptr::null_mut();
    }
    dbg("Converted swapchain ptr to vtable ptr");
    dbg(&format!(
        "original swapchain={sc:p}, vtbl_ptr={vtbl_ptr:p}"
    ));

    // Step 5: IDXGISwapChain::Present = vtable slot 8 (0-based)
    let present = *vtbl_ptr.add(8);
    if present.is_null() {
        dbg("Present pointer is null");
        return core::ptr::null_mut();
    }
    dbg(&format!("get_present_addr_from_swapchain: Present = {present:p}"));
    present as *mut c_void
}

// Replace the Present pointer in the swapchain vtable with our hook
// unsafe fn hook_swapchain_present(
//     swapchain_raw: *mut core::ffi::c_void,
// ) -> Result<(), &'static str> {
//     if swapchain_raw.is_null() {
//         dbg("hook_swapchain_present: swapchain null");
//         return Err("swapchain null");
//     }

//     // A COM object pointer layout: first field is pointer to vtable
//     let vtable_ptr_ptr = swapchain_raw as *mut *mut *mut core::ffi::c_void;
//     if vtable_ptr_ptr.is_null() {
//         dbg("hook_swapchain_present: vtable_ptr_ptr null");
//         return Err("vtable_ptr_ptr null");
//     }
//     let vtable = *vtable_ptr_ptr; // *mut *mut c_void
//     if vtable.is_null() {
//         dbg("hook_swapchain_present: vtable null");
//         return Err("vtable null");
//     }

//     // Present is usually vtable index 8
//     const PRESENT_INDEX: isize = 8;
//     let present_slot = vtable.offset(PRESENT_INDEX);

//     // Read original function pointer
//     let orig_present_ptr = *present_slot as *mut core::ffi::c_void;
//     ORIG_PRESENT.store(orig_present_ptr, Ordering::SeqCst);

//     // Change memory protection to writable. VirtualProtect expects a pointer to PAGE_PROTECTION_FLAGS
//     let mut old_protect = PAGE_PROTECTION_FLAGS(0);
//     let size = mem::size_of::<*mut core::ffi::c_void>();

//     // VirtualProtect returns BOOL wrapped in windows crate -> use .as_bool() or check != 0
//     let ok0 = VirtualProtect(
//         present_slot as *mut core::ffi::c_void,
//         size,
//         PAGE_EXECUTE_READWRITE,
//         &mut old_protect,
//     );
//     if ok0 == BOOL(0) {
//         dbg("VirtualProtect (set RW) failed");
//         return Err("VirtualProtect (set RW) failed");
//     }

//     // Write our Present pointer
//     // NOTE: cast to pointer types carefully
//     let mine = my_present as *const () as *mut core::ffi::c_void;
//     *present_slot = mine;

//     // Restore old protection
//     let ok1 = VirtualProtect(
//         present_slot as *mut core::ffi::c_void,
//         size,
//         old_protect,
//         &mut old_protect,
//     );
//     if ok1 == BOOL(0) {
//         dbg("VirtualProtect (restore) failed");
//         return Err("VirtualProtect (restore) failed");
//     }

//     Ok(())
// }

#[no_mangle]
pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        dbg("DllMain: DLL_PROCESS_ATTACH");
        unsafe { let _ = DisableThreadLibraryCalls(hinst); }

        std::thread::spawn(|| {
            dbg("init_thread: started");
            install_panic_hook();
            dbg("init_thread: panic hook installed");
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


// --- globals ---------------------------------------------------
static START: Once = Once::new();

type PFN_D3D11CreateDevice = extern "system" fn(
    pAdapter: *mut c_void,                 // IDXGIAdapter*
    DriverType: D3D_DRIVER_TYPE,           // or u32
    Software: *mut c_void,                 // HMODULE
    Flags: u32,
    pFeatureLevels: *const D3D_FEATURE_LEVEL,
    FeatureLevels: u32,
    SDKVersion: u32,
    ppDevice: *mut *mut ID3D11Device,
    pFeatureLevel: *mut D3D_FEATURE_LEVEL,
    ppImmediateContext: *mut *mut ID3D11DeviceContext,
) -> HRESULT;

type PFN_D3D11CreateDeviceAndSwapChain = extern "system" fn(
    pAdapter: *mut c_void,
    DriverType: D3D_DRIVER_TYPE,
    Software: *mut c_void,
    Flags: u32,
    pFeatureLevels: *const D3D_FEATURE_LEVEL,
    FeatureLevels: u32,
    SDKVersion: u32,
    pSwapChainDesc: *mut DXGI_SWAP_CHAIN_DESC,
    ppSwapChain: *mut *mut IDXGISwapChain,
    ppDevice: *mut *mut ID3D11Device,
    pFeatureLevel: *mut D3D_FEATURE_LEVEL,
    ppImmediateContext: *mut *mut ID3D11DeviceContext,
) -> HRESULT;

type PFN_IDXGIFactoryCreateSwapChain = extern "system" fn(
    this: *mut IDXGIFactory,
    pDevice: *mut c_void,
    pDesc: *mut DXGI_SWAP_CHAIN_DESC,
    ppSwapChain: *mut *mut IDXGISwapChain,
) -> HRESULT;

type PFN_IDXGIFactory2_CreateSwapChainForHwnd = extern "system" fn(
    this: *mut windows::Win32::Graphics::Dxgi::IDXGIFactory2,
    pDevice: *mut core::ffi::c_void,         // IUnknown*
    hWnd: HWND,
    pDesc: *const DXGI_SWAP_CHAIN_DESC1,
    pFullscreen: *const DXGI_SWAP_CHAIN_FULLSCREEN_DESC,
    pRestrictToOutput: *mut IDXGIOutput,
    ppSwapChain: *mut *mut IDXGISwapChain1,
) -> windows::core::HRESULT;

type PFN_IDXGIFactory2_CreateSwapChainForCoreWindow = extern "system" fn(
    this: *mut IDXGIFactory2,
    pDevice: *mut c_void,
    pWindow: *mut c_void, // IUnknown (CoreWindow)
    pDesc: *const DXGI_SWAP_CHAIN_DESC1,
    pRestrictToOutput: *mut IDXGIOutput,
    ppSwapChain: *mut *mut IDXGISwapChain1,
) -> HRESULT;

type PFN_IDXGIFactory2_CreateSwapChainForComposition = extern "system" fn(
    this: *mut IDXGIFactory2,
    pDevice: *mut c_void,
    pDesc: *const DXGI_SWAP_CHAIN_DESC1,
    pRestrictToOutput: *mut IDXGIOutput,
    ppSwapChain: *mut *mut IDXGISwapChain1,
) -> HRESULT;

type PFN_CreateDXGIFactory1 = extern "system" fn(riid: *const windows::core::GUID, ppFactory: *mut *mut c_void) -> HRESULT;
type PFN_CreateDXGIFactory2 = extern "system" fn(Flags: u32, riid: *const windows::core::GUID, ppFactory: *mut *mut c_void) -> HRESULT;

static mut REAL_D3D11_CREATE_DEVICE_AND_SWAP_CHAIN: Option<PFN_D3D11CreateDeviceAndSwapChain> = None;
static mut REAL_D3D11_CREATE_DEVICE: Option<PFN_D3D11CreateDevice> = None;
static mut REAL_IDXGIFACTORY_CREATE_SWAPCHAIN: Option<PFN_IDXGIFactoryCreateSwapChain> = None;
static mut REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_HWND: Option<PFN_IDXGIFactory2_CreateSwapChainForHwnd> = None;
static mut REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COREWINDOW: Option<PFN_IDXGIFactory2_CreateSwapChainForCoreWindow> = None;
static mut REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COMPOSITION: Option<PFN_IDXGIFactory2_CreateSwapChainForComposition> = None;
static mut REAL_CREATE_DXGI_FACTORY1: Option<PFN_CreateDXGIFactory1> = None;

// --- simple helpers --------------------------------------------


#[no_mangle]
pub extern "system" fn my_d3d11_create_device_and_swap_chain(
    pAdapter: *mut c_void,
    DriverType: D3D_DRIVER_TYPE,
    Software: *mut c_void,
    Flags: u32,
    pFeatureLevels: *const D3D_FEATURE_LEVEL,
    FeatureLevels: u32,
    SDKVersion: u32,
    pSwapChainDesc: *mut DXGI_SWAP_CHAIN_DESC,
    ppSwapChain: *mut *mut IDXGISwapChain,
    ppDevice: *mut *mut ID3D11Device,
    pFeatureLevel: *mut D3D_FEATURE_LEVEL,
    ppImmediateContext: *mut *mut ID3D11DeviceContext,
) -> HRESULT {
    dbg("[hook] my_d3d11_create_device_and_swap_chain called");
    let real = unsafe { REAL_D3D11_CREATE_DEVICE_AND_SWAP_CHAIN.expect("real d3d11 fn not set") };

    let hr = real(
        pAdapter,
        DriverType,
        Software,
        Flags,
        pFeatureLevels,
        FeatureLevels,
        SDKVersion,
        pSwapChainDesc,
        ppSwapChain,
        ppDevice,
        pFeatureLevel,
        ppImmediateContext,
    );

    if hr.is_ok() {
        unsafe {
            if !ppSwapChain.is_null() && !(*ppSwapChain).is_null() {
                // Clone takes an AddRef’d smart wrapper in windows-rs
                RAW_SWAPCHAIN = *ppSwapChain as *mut c_void;

                
                if !PRESENT_HOOKED.swap(true, Ordering::SeqCst) {
                    let present_addr = get_present_addr_from_swapchain(*ppSwapChain);
                    if !present_addr.is_null() {
                        if let Some(orig) = minhook_init_and_hook(present_addr, my_present as *mut _) {
                            ORIG_PRESENT.store(orig, std::sync::atomic::Ordering::SeqCst);
                            dbg(&format!("[hook] Present hook installed successfully at {:p}", present_addr));
                        } else {
                            dbg("[hook] Failed to install Present hook!");
                        }
                    } else {
                        dbg("[hook] Failed to get Present address");
                    }
                }
            }
            if !ppDevice.is_null() && !(*ppDevice).is_null() {
                RAW_DEVICE = *ppDevice as *mut c_void;
            }
            if !ppImmediateContext.is_null() && !(*ppImmediateContext).is_null() {
                RAW_DEVICE_CTX = *ppImmediateContext as *mut c_void;
            }
        }
    }

    hr
}

#[no_mangle]
pub extern "system" fn my_d3d11_create_device(
    pAdapter: *mut c_void,
    DriverType: D3D_DRIVER_TYPE,
    Software: *mut c_void,
    Flags: u32,
    pFeatureLevels: *const D3D_FEATURE_LEVEL,
    FeatureLevels: u32,
    SDKVersion: u32,
    ppDevice: *mut *mut ID3D11Device,
    pFeatureLevel: *mut D3D_FEATURE_LEVEL,
    ppImmediateContext: *mut *mut ID3D11DeviceContext,
) -> HRESULT {
    dbg("[hook] D3D11CreateDevice called!");
    let real = unsafe { REAL_D3D11_CREATE_DEVICE.unwrap() };
    let hr = real(
        pAdapter, DriverType, Software, Flags,
        pFeatureLevels, FeatureLevels, SDKVersion,
        ppDevice, pFeatureLevel, ppImmediateContext,
    );
    dbg("[hook] D3D11CreateDevice real constructed");

    if hr.is_ok() {
        unsafe {
            if !ppDevice.is_null() && !(*ppDevice).is_null() {
                RAW_DEVICE = *ppDevice as *mut std::ffi::c_void;
                GLOBAL_DEVICE = Some(wrap_device_from_raw(RAW_DEVICE));
                dbg("[hook] Found and bound global device");
            }
            if !ppImmediateContext.is_null() && !(*ppImmediateContext).is_null() {
                RAW_DEVICE_CTX = *ppImmediateContext as *mut std::ffi::c_void;
                GLOBAL_DEVICE_CTX = Some(wrap_device_context_from_raw(RAW_DEVICE_CTX));
                dbg("[hook] Found and bound global device context");
            }
        }
        dbg("[hook] captured device + immediate context");
    }
    hr
}

#[no_mangle]
pub extern "system" fn my_create_dxgi_factory1(
    riid: *const windows::core::GUID,
    ppFactory: *mut *mut c_void,
) -> HRESULT {
    dbg("[hook] CreateDXGIFactory1 called!");
    let real = unsafe { REAL_CREATE_DXGI_FACTORY1.unwrap() };
    let hr = real(riid, ppFactory);

    if hr.is_ok() {
        unsafe {
            install_global_factory_hooks();
            if !ppFactory.is_null() && !(*ppFactory).is_null() {
                // Treat returned interface as IDXGIFactory* (base of IDXGIFactory1)
                let factory = *ppFactory as *mut IDXGIFactory;
                let addr = get_factory_create_swapchain_addr(factory);

                if !addr.is_null() {
                    if let Some(orig) = minhook_init_and_hook(addr, my_idxgifactory_create_swap_chain as *mut _) {
                        REAL_IDXGIFACTORY_CREATE_SWAPCHAIN =
                            Some(std::mem::transmute::<*mut c_void, PFN_IDXGIFactoryCreateSwapChain>(orig));
                        dbg("[hook] Hooked IDXGIFactory::CreateSwapChain (vtable slot 10)");
                    } else {
                        dbg("[hook] Failed to hook IDXGIFactory::CreateSwapChain");
                    }
                }
            }
        }
    }
    hr
}

#[no_mangle]
pub extern "system" fn my_idxgifactory_create_swap_chain(
    this: *mut IDXGIFactory,
    pDevice: *mut c_void,
    pDesc: *mut DXGI_SWAP_CHAIN_DESC,
    ppSwapChain: *mut *mut IDXGISwapChain,
) -> HRESULT {
    dbg("[hook] IDXGIFactory::CreateSwapChain called");
    let real = unsafe { REAL_IDXGIFACTORY_CREATE_SWAPCHAIN.expect("REAL_IDXGIFACTORY_CREATE_SWAPCHAIN not set") };
    let hr = real(this, pDevice, pDesc, ppSwapChain);

    if hr.is_ok() {
        unsafe {
            if !ppSwapChain.is_null() && !(*ppSwapChain).is_null() {
                RAW_SWAPCHAIN = *ppSwapChain as *mut c_void;
                dbg("[hook] captured real IDXGISwapChain from CreateSwapChain");

                // Install Present hook once, from the real swapchain vtable
                if !PRESENT_HOOKED.swap(true, Ordering::SeqCst) {
                    let present_addr = get_present_addr_from_swapchain(*ppSwapChain);
                    if !present_addr.is_null() {
                        if let Some(orig) = minhook_init_and_hook(present_addr, my_present as *mut _) {
                            ORIG_PRESENT.store(orig, std::sync::atomic::Ordering::SeqCst);
                            dbg(&format!("[hook] Present hooked at {:p}", present_addr));
                        } else {
                            dbg("[hook] Failed to hook Present");
                        }
                    } else {
                        dbg("[hook] Present address was null");
                    }
                }
            }
        }
    }

    hr
}

#[no_mangle]
pub extern "system" fn my_factory2_create_swap_chain_for_hwnd(
    this: *mut IDXGIFactory2,
    pDevice: *mut c_void,
    hWnd: windows::Win32::Foundation::HWND,
    pDesc: *const DXGI_SWAP_CHAIN_DESC1,
    pFullscreenDesc: *const DXGI_SWAP_CHAIN_FULLSCREEN_DESC,
    pRestrictToOutput: *mut IDXGIOutput,
    ppSwapChain: *mut *mut IDXGISwapChain1,
) -> HRESULT {
    dbg("[hook] IDXGIFactory2::CreateSwapChainForHwnd called");
    let real = unsafe {
        REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_HWND
            .expect("REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_HWND not set")
    };
    let hr = real(
        this,
        pDevice,
        hWnd,
        pDesc,
        pFullscreenDesc,
        pRestrictToOutput,
        ppSwapChain,
    );
    if hr.is_ok() {
        unsafe { handle_created_swapchain(ppSwapChain) };
    }
    hr
}

#[no_mangle]
pub extern "system" fn my_factory2_create_swap_chain_for_corewindow(
    this: *mut IDXGIFactory2,
    pDevice: *mut c_void,
    pWindow: *mut c_void,
    pDesc: *const DXGI_SWAP_CHAIN_DESC1,
    pRestrictToOutput: *mut IDXGIOutput,
    ppSwapChain: *mut *mut IDXGISwapChain1,
) -> HRESULT {
    dbg("[hook] IDXGIFactory2::CreateSwapChainForCoreWindow called");
    let real = unsafe {
        REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COREWINDOW
            .expect("REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COREWINDOW not set")
    };
    let hr = real(this, pDevice, pWindow, pDesc, pRestrictToOutput, ppSwapChain);
    if hr.is_ok() {
        unsafe { handle_created_swapchain(ppSwapChain) };
    }
    hr
}

#[no_mangle]
pub extern "system" fn my_factory2_create_swap_chain_for_composition(
    this: *mut IDXGIFactory2,
    pDevice: *mut c_void,
    pDesc: *const DXGI_SWAP_CHAIN_DESC1,
    pRestrictToOutput: *mut IDXGIOutput,
    ppSwapChain: *mut *mut IDXGISwapChain1,
) -> HRESULT {
    dbg("[hook] IDXGIFactory2::CreateSwapChainForComposition called");
    let real = unsafe {
        REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COMPOSITION
            .expect("REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COMPOSITION not set")
    };
    let hr = real(this, pDevice, pDesc, pRestrictToOutput, ppSwapChain);
    if hr.is_ok() {
        unsafe { handle_created_swapchain(ppSwapChain) };
    }
    hr
}

unsafe fn handle_created_swapchain(pp_swap_chain: *mut *mut IDXGISwapChain1) {
    if pp_swap_chain.is_null() || (*pp_swap_chain).is_null() {
        dbg("[hook] swapchain pointer is null, skipping");
        return;
    }

    // Cast IDXGISwapChain1 → IDXGISwapChain (they share base layout)
    let sc = *pp_swap_chain as *mut IDXGISwapChain;
    RAW_SWAPCHAIN = *pp_swap_chain as *mut c_void;

    if !PRESENT_HOOKED.swap(true, Ordering::SeqCst) {
        let present_addr = get_present_addr_from_swapchain(sc);
        if !present_addr.is_null() {
            if let Some(orig) = minhook_init_and_hook(present_addr, my_present as *mut _) {
                ORIG_PRESENT.store(orig, Ordering::SeqCst);
                dbg(&format!("[hook] Present hooked at {:p}", present_addr));
            } else {
                dbg("[hook] Failed to hook Present");
            }
        } else {
            dbg("[hook] Present addr was null");
        }
    }
}


#[inline]
unsafe fn get_factory_create_swapchain_addr(factory: *mut IDXGIFactory) -> *mut c_void {
    // COM object: first field is vtable pointer
    let vtbl = *(factory as *const *const *const c_void);
    // IDXGIFactory::CreateSwapChain is slot 10 (0-based)
    *vtbl.add(10) as *mut c_void
}

unsafe fn ensure_minhook_init() {
    if !MH_INIT_DONE.swap(true, Ordering::SeqCst) {
        if MH_Initialize() != 0 { dbg("MH_Initialize failed"); }
        dbg("MH_Initialize ok");
    }
}

unsafe fn try_install_d3d_hooks_if_modules_present() {
    if HOOKS_INSTALLED_FOR_D3D.load(Ordering::SeqCst) { return; }

    let d3d11_opt = GetModuleHandleA(PCSTR(b"d3d11.dll\0".as_ptr())).ok();
    let dxgi_opt  = GetModuleHandleA(PCSTR(b"dxgi.dll\0".as_ptr())).ok();
    if d3d11_opt.is_none() || dxgi_opt.is_none() { return; }

    let d3d11 = d3d11_opt.unwrap();
    let dxgi  = dxgi_opt.unwrap();

    let addr_create_dev   = GetProcAddress(d3d11, PCSTR(b"D3D11CreateDevice\0".as_ptr()));
    let addr_create_devsc = GetProcAddress(d3d11, PCSTR(b"D3D11CreateDeviceAndSwapChain\0".as_ptr()));
    let addr_factory1     = GetProcAddress(dxgi,  PCSTR(b"CreateDXGIFactory1\0".as_ptr()));

    let addr_create_dev   = addr_create_dev.map(|f| f as *mut c_void).unwrap_or(std::ptr::null_mut());
    let addr_create_devsc = addr_create_devsc.map(|f| f as *mut c_void).unwrap_or(std::ptr::null_mut());
    let addr_factory1     = addr_factory1.map(|f| f as *mut c_void).unwrap_or(std::ptr::null_mut());

    ensure_minhook_init();

    dbg(&format!(
        "[hook] Resolved exports: dev={addr_create_dev:p} devsc={addr_create_devsc:p} factory1={addr_factory1:p}"
    ));

    if !addr_create_dev.is_null() {
        if let Some(original_create_dev) = minhook_init_and_hook(addr_create_dev, my_d3d11_create_device as *mut _) {
            REAL_D3D11_CREATE_DEVICE = Some(std::mem::transmute::<*mut c_void, PFN_D3D11CreateDevice>(original_create_dev));
            dbg("[hook] Hooked D3D11CreateDevice");
        }
    }
    if !addr_create_devsc.is_null() {
        if let Some(original_create_devsc) = minhook_init_and_hook(addr_create_devsc, my_d3d11_create_device_and_swap_chain as *mut _) {
            REAL_D3D11_CREATE_DEVICE_AND_SWAP_CHAIN = Some(std::mem::transmute::<*mut c_void, PFN_D3D11CreateDeviceAndSwapChain>(original_create_devsc));
            dbg("[hook] Hooked D3D11CreateDeviceAndSwapChain");
        }
    }
    if !addr_factory1.is_null() {
        if let Some(original_factory1) = minhook_init_and_hook(addr_factory1, my_create_dxgi_factory1 as *mut _) {
            REAL_CREATE_DXGI_FACTORY1 = Some(std::mem::transmute(original_factory1));
            dbg("[hook] Hooked CreateDXGIFactory1");
        }
    }
    HOOKS_INSTALLED_FOR_D3D.store(true, Ordering::SeqCst);
}

/// Get the vtable function address at `index` for a COM object pointer.
/// `obj` must be a COM interface pointer (`*mut T` where T is a windows-rs interface).
#[inline]
unsafe fn vtbl_entry<T>(obj: *mut T, index: usize) -> *mut c_void {
    // COM object layout: [ vtable_ptr | ... ]
    // vtable_ptr is `*const *const c_void` (array of function pointers).
    let vtbl: *const *const c_void = *(obj as *mut *const *const c_void);
    // Pick the function pointer at the given slot and cast to *mut c_void for MinHook.
    *vtbl.add(index) as *mut c_void
}


unsafe fn install_global_factory_hooks() {
    // Create a tiny dummy factory via the REAL trampoline
    let mut pf: *mut c_void = std::ptr::null_mut();
    let real = REAL_CREATE_DXGI_FACTORY1.expect("real CreateDXGIFactory1 not set");
    let hr = real(&IDXGIFactory::IID, &mut pf);
    if hr.is_err() || pf.is_null() {
        dbg("[hook] dummy IDXGIFactory creation failed");
        return;
    }
    let factory = pf as *mut IDXGIFactory;

    let addr_create_sc = vtbl_entry(factory, 10);
    if !addr_create_sc.is_null() {
        if let Some(orig) = minhook_init_and_hook(addr_create_sc, my_idxgifactory_create_swap_chain as *mut _) {
            REAL_IDXGIFACTORY_CREATE_SWAPCHAIN = Some(std::mem::transmute::<*mut c_void, PFN_IDXGIFactoryCreateSwapChain>(orig));
            dbg("[hook] globally hooked IDXGIFactory::CreateSwapChain");
        }
    }

    // ---- 2) Try CreateDXGIFactory2 export to get a real IDXGIFactory2 (no QueryInterface)
    let dxgi = match GetModuleHandleA(PCSTR(b"dxgi.dll\0".as_ptr())).ok() {
        Some(h) => h,
        None => {
            dbg("[hook] dxgi.dll not loaded yet (unexpected here)");
            return;
        }
    };
    dbg("[hook] loaded dxgi.dll");
    let pfn_cdf2 = GetProcAddress(dxgi, PCSTR(b"CreateDXGIFactory2\0".as_ptr()))
        .map(|f| f as *const ())
        .unwrap_or(std::ptr::null());

    if pfn_cdf2.is_null() {
        dbg("[hook] CreateDXGIFactory2 not exported on this system; skipping Factory2 hooks");
        return;
    }
    dbg("[hook] Got CreateDXGIFactory2 raw");

    let create_dxgi_factory2: PFN_CreateDXGIFactory2 = std::mem::transmute(pfn_cdf2);
    dbg("[hook] Transmuted CreateDXGIFactory2");

    let mut pf2: *mut c_void = std::ptr::null_mut();
    // Flags=0 is fine; we just need a COM object
    let hr2 = create_dxgi_factory2(0, &IDXGIFactory2::IID, &mut pf2);
    if hr2.is_err() || pf2.is_null() {
        dbg("[hook] CreateDXGIFactory2 failed; skipping Factory2 hooks");
        return;
    }
    dbg("[hook] created dxgi factor2");
    
    // Get a raw pointer to IDXGIFactory2 (use transmute_copy for windows 0.36 compatibility)
    let f2_iface: IDXGIFactory2 = transmute_copy(&pf2); // constructs wrapper from raw
    dbg("[hook] transmuted f2_iface");
    let f2_ptr: *mut IDXGIFactory2 = transmute_copy(&f2_iface); // extract raw pointer
    dbg("[hook] transmuted f2_ptr");

    let addr_for_hwnd       = vtbl_entry(f2_ptr, 15);
    let addr_for_corewindow = vtbl_entry(f2_ptr, 16);
    let addr_for_composition= vtbl_entry(f2_ptr, 24);

    if !addr_for_hwnd.is_null() {
        if let Some(orig) = minhook_init_and_hook(addr_for_hwnd, my_factory2_create_swap_chain_for_hwnd as *mut _) {
            REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_HWND = Some(std::mem::transmute(orig));
            dbg("[hook] globally hooked IDXGIFactory2::CreateSwapChainForHwnd");
        }
    }
    if !addr_for_corewindow.is_null() {
        if let Some(orig) = minhook_init_and_hook(addr_for_corewindow, my_factory2_create_swap_chain_for_corewindow as *mut _) {
            REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COREWINDOW = Some(std::mem::transmute(orig));
            dbg("[hook] globally hooked IDXGIFactory2::CreateSwapChainForCoreWindow");
        }
    }
    if !addr_for_composition.is_null() {
        if let Some(orig) = minhook_init_and_hook(addr_for_composition, my_factory2_create_swap_chain_for_composition as *mut _) {
            REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COMPOSITION = Some(std::mem::transmute(orig));
            dbg("[hook] globally hooked IDXGIFactory2::CreateSwapChainForComposition");
        }
    }
    dbg("[hook] Finished IDXGIFactory2 variants");
}



// originals
static mut REAL_LOADLIB_A: Option<extern "system" fn(PCSTR) -> *mut c_void> = None;
static mut REAL_LOADLIB_W: Option<extern "system" fn(PCWSTR) -> *mut c_void> = None;

// detours
#[no_mangle]
pub extern "system" fn my_LoadLibraryA(name: PCSTR) -> *mut c_void {
    let real = unsafe { REAL_LOADLIB_A.expect("REAL_LOADLIB_A") };
    let h = real(name);
    // After any library load, try to install d3d hooks (idempotent)
    unsafe { try_install_d3d_hooks_if_modules_present(); }
    h
}

#[no_mangle]
pub extern "system" fn my_LoadLibraryW(name: PCWSTR) -> *mut c_void {
    let real = unsafe { REAL_LOADLIB_W.expect("REAL_LOADLIB_W") };
    let h = real(name);
    unsafe { try_install_d3d_hooks_if_modules_present(); }
    h
}

#[ctor::ctor]
fn init() {
    unsafe {
        ensure_minhook_init();

        // Resolve kernel32 LoadLibrary exports.
        let k32 = GetModuleHandleA(PCSTR(b"kernel32.dll\0".as_ptr())).ok();
        if let Some(k32) = k32 {
            let addr_ll_a = GetProcAddress(k32, PCSTR(b"LoadLibraryA\0".as_ptr())).map(|f| f as *mut c_void).unwrap_or(std::ptr::null_mut());
            let addr_ll_w = GetProcAddress(k32, PCSTR(b"LoadLibraryW\0".as_ptr())).map(|f| f as *mut c_void).unwrap_or(std::ptr::null_mut());

            if !addr_ll_a.is_null() {
                if let Some(orig) = minhook_init_and_hook(addr_ll_a, my_LoadLibraryA as *mut _) {
                    REAL_LOADLIB_A = Some(std::mem::transmute(orig));
                    dbg("[hook] Hooked LoadLibraryA");
                }
            }
            if !addr_ll_w.is_null() {
                if let Some(orig) = minhook_init_and_hook(addr_ll_w, my_LoadLibraryW as *mut _) {
                    REAL_LOADLIB_W = Some(std::mem::transmute(orig));
                    dbg("[hook] Hooked LoadLibraryW");
                }
            }
        }

        // In case d3d11/dxgi were already loaded before our DLL, try now too:
        try_install_d3d_hooks_if_modules_present();
    }
}


static MASHER_ACTIVE: AtomicBool = AtomicBool::new(false);
static INIT_SOCKET: Once = Once::new();

fn start_pipe_thread() {
    INIT_SOCKET.call_once(|| {
        thread::spawn(|| {
            // panic hook for safety
            std::panic::set_hook(Box::new(|info| {
                eprintln!("Pipe listener panicked: {info}");
            }));

            let pipe_name = r"\\.\pipe\masher_overlay";
            let name_w: Vec<u16> = OsStr::new(pipe_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            dbg("Set up overlay pipe");

            loop {
                unsafe {
                    let handle = CreateNamedPipeW(
                        PCWSTR(name_w.as_ptr()),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        512,
                        512,
                        0,
                        ptr::null_mut(),
                    );

                    if handle == INVALID_HANDLE_VALUE {
                        eprintln!("Failed to create named pipe");
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }

                    println!("Masher overlay pipe listener started");

                    let connected = ConnectNamedPipe(handle, ptr::null_mut());
                    if connected.as_bool() == false {
                        let err = GetLastError();
                        if err != ERROR_PIPE_CONNECTED {
                            eprintln!("ConnectNamedPipe failed: {}", err.0);
                            thread::sleep(Duration::from_secs(1));
                            continue;
                        }
                    }

                    println!("Client connected!");

                    let mut buffer = [0u8; 128];
                    let mut bytes_read = 0u32;
                    let success = ReadFile(
                        handle,
                        buffer.as_mut_ptr() as *mut _,
                        buffer.len() as u32,
                        &mut bytes_read,
                        ptr::null_mut(),
                    );

                    if success.as_bool() && bytes_read > 0 {
                        let cmd = String::from_utf8_lossy(&buffer[..bytes_read as usize]);
                        dbg(&format!("Received pipe command {}", cmd.trim()));
                        match cmd.trim() {
                            "masher_active" => {
                                MASHER_ACTIVE.store(true, Ordering::SeqCst);
                                println!("Masher active!");
                            }
                            "masher_inactive" => {
                                MASHER_ACTIVE.store(false, Ordering::SeqCst);
                                println!("Masher inactive!");
                            }
                            other => println!("Unknown command: {other}"),
                        }
                    }

                    FlushFileBuffers(handle);
                    DisconnectNamedPipe(handle);
                    // Loop back to accept next client

                    println!("Client disconnected!");
                }
            }
        });
    });
}