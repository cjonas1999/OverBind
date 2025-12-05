// Cargo.toml must set crate-type = ["cdylib"]
// target: i686-pc-windows-msvc (32-bit)

#![allow(non_snake_case)]
use imgui::{Condition, Context as ImGuiContext};
use imgui_dx11_renderer::Renderer;
use minhook_sys::{MH_CreateHook, MH_EnableHook, MH_Initialize};
use std::ffi::{c_void, CString, OsStr};
use std::mem::{transmute_copy, ManuallyDrop};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{self, null, null_mut, NonNull};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicU32, Ordering};
use std::sync::{Once, OnceLock};
use std::time::Duration;
use std::{mem, panic, thread};
use widestring::U16CString;
use windows::core::Interface;
use windows::core::{Error, Result as WinResult, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, BOOL, ERROR_PIPE_CONNECTED, E_FAIL, FARPROC, HINSTANCE, HWND,
    INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceChild,
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_MODE_DESC, DXGI_MODE_SCALING_UNSPECIFIED, DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    Common::DXGI_FORMAT_R8G8B8A8_UNORM, IDXGISwapChain, DXGI_SWAP_CHAIN_DESC,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIFactory, IDXGIFactory2, IDXGIOutput, IDXGISwapChain1, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FULLSCREEN_DESC, DXGI_SWAP_EFFECT_DISCARD,
};
use windows::Win32::Storage::FileSystem::{
    FlushFileBuffers, ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, GetModuleHandleA, GetProcAddress, LoadLibraryExA, LoadLibraryW,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::*;

type HMODULE = isize;

// Global place to store original Present pointer
static ORIG_PRESENT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
// Flag to initialized overlay once
static INIT_IMGUI: Once = Once::new();
static mut GLOBAL_CTX: Option<ImGuiContext> = None;
static mut GLOBAL_RENDERER: Option<Renderer> = None;
static mut DEFAULT_FONT: Option<imgui::FontId> = None;
static mut ICON_FONT: Option<imgui::FontId> = None;
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
static mut LAST_VIEWPORT: D3D11_VIEWPORT = unsafe {std::mem::zeroed()};

unsafe fn borrow_device() -> Option<&'static windows::Win32::Graphics::Direct3D11::ID3D11Device> {
    if RAW_DEVICE.is_null() {
        return None;
    }
    Some(&*(RAW_DEVICE as *mut windows::Win32::Graphics::Direct3D11::ID3D11Device))
}

unsafe fn wrap_device_from_raw(raw: *mut std::ffi::c_void) -> ManuallyDrop<ID3D11Device> {
    let dev: ID3D11Device = transmute_copy(&raw);
    ManuallyDrop::new(dev) // prevent Drop => no Release
}

unsafe fn wrap_device_context_from_raw(
    raw: *mut std::ffi::c_void,
) -> ManuallyDrop<ID3D11DeviceContext> {
    let ctx: ID3D11DeviceContext = transmute_copy(&raw);
    ManuallyDrop::new(ctx) // prevent Drop => no Release
}

unsafe fn wrap_swapchain_from_raw(raw: *mut std::ffi::c_void) -> ManuallyDrop<IDXGISwapChain> {
    let sc: IDXGISwapChain = transmute_copy(&raw);
    ManuallyDrop::new(sc) // prevent Drop => no Release
}

fn get_viewport_size(ctx: &ID3D11DeviceContext) -> (f32, f32) {
    unsafe {
        let mut vp = LAST_VIEWPORT;
        let mut count = 1u32;
        ctx.RSGetViewports(&mut count, &mut vp);
        if vp.Width != LAST_VIEWPORT.Width || vp.Height != LAST_VIEWPORT.Height {
            LAST_VIEWPORT = vp;
        }
        (LAST_VIEWPORT.Width, LAST_VIEWPORT.Height)
    }
}

// Our Present hook (matching IDXGISwapChain::Present signature)
extern "system" fn my_present(this: *mut core::ffi::c_void, sync: u32, flags: u32) -> HRESULT {
    unsafe {
        if RAW_DEVICE.is_null() || RAW_DEVICE_CTX.is_null() || RAW_SWAPCHAIN.is_null() {
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

                    let default_font = fonts.add_font(&[imgui::FontSource::DefaultFontData {
                        config: Some(imgui::FontConfig {
                            size_pixels: 12.0,
                            ..Default::default()
                        }),
                    }]);
                    let font_data = include_bytes!("../../../src-tauri/icons/IconFont.ttf");
                    let icon_font = fonts.add_font(&[imgui::FontSource::TtfData {
                        data: font_data,
                        size_pixels: 64.0,
                        config: Some(imgui::FontConfig {
                            size_pixels: 64.0,
                            ..Default::default()
                        }),
                    }]);

                    DEFAULT_FONT = Some(default_font);
                    ICON_FONT = Some(icon_font);
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
                        dbg(&format!(
                            "[overlay] Renderer::new failed in CreateDevice: {:?}",
                            e
                        ));
                    }
                }

                start_pipe_thread();
            }
        }
        let mut w = 0f32;
        let mut h: f32 = 0f32;
        if let Some(device_ctx) = GLOBAL_DEVICE_CTX.as_ref() {
            (w, h) = get_viewport_size(device_ctx);
        }

        // ImGui render
        {
            let ctx = GLOBAL_CTX.as_mut().unwrap();
            let io = ctx.io_mut();
            io.display_size = [w as f32, h as f32];

            let ui = ctx.frame();
            {
                // Get the foreground draw list (always rendered on top)
                let draw_list = ui.get_foreground_draw_list();

                let active = MASHER_ACTIVE.load(Ordering::SeqCst);

                let color = if active {
                    imgui::ImColor32::from_rgba(255, 0, 0, 255) // red
                } else {
                    imgui::ImColor32::from_rgba(100, 100, 100, 150) // dim gray
                };

                if let Some(font) = ICON_FONT {
                    let _font_token = ui.push_font(font);
                    draw_list.add_text([20.0, 60.0], color, "X");
                }

                if let Some(font) = DEFAULT_FONT {
                    let _font_token = ui.push_font(font);
                    draw_list.add_text([20.0, 100.0], color, "v2.0.2-beta");
                }
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

pub unsafe fn minhook_init_and_hook(
    target: *mut c_void,
    detour: *mut c_void,
) -> Option<*mut c_void> {
    dbg(&format!(
        "minhook_init_and_hook: target={target:p}, detour={detour:p}"
    ));

    let mut orig: *mut c_void = std::ptr::null_mut();
    let r1 = MH_CreateHook(target, detour, &mut orig as *mut _ as *mut _);
    dbg(&format!("MH_CreateHook: {r1}"));
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
    dbg(&format!("original swapchain={sc:p}, vtbl_ptr={vtbl_ptr:p}"));

    // Step 5: IDXGISwapChain::Present = vtable slot 8 (0-based)
    let present = *vtbl_ptr.add(8);
    if present.is_null() {
        dbg("Present pointer is null");
        return core::ptr::null_mut();
    }
    dbg(&format!(
        "get_present_addr_from_swapchain: Present = {present:p}"
    ));
    present as *mut c_void
}

#[no_mangle]
pub extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        dbg("DllMain: DLL_PROCESS_ATTACH");
        unsafe {
            let _ = DisableThreadLibraryCalls(hinst);
        }

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
    pAdapter: *mut c_void,       // IDXGIAdapter*
    DriverType: D3D_DRIVER_TYPE, // or u32
    Software: *mut c_void,       // HMODULE
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
    pDevice: *mut core::ffi::c_void, // IUnknown*
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

type PFN_CreateDXGIFactory1 =
    extern "system" fn(riid: *const windows::core::GUID, ppFactory: *mut *mut c_void) -> HRESULT;
type PFN_CreateDXGIFactory2 = extern "system" fn(
    Flags: u32,
    riid: *const windows::core::GUID,
    ppFactory: *mut *mut c_void,
) -> HRESULT;

static mut REAL_D3D11_CREATE_DEVICE_AND_SWAP_CHAIN: Option<PFN_D3D11CreateDeviceAndSwapChain> =
    None;
static mut REAL_D3D11_CREATE_DEVICE: Option<PFN_D3D11CreateDevice> = None;
static mut REAL_IDXGIFACTORY_CREATE_SWAPCHAIN: Option<PFN_IDXGIFactoryCreateSwapChain> = None;
static mut REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_HWND: Option<
    PFN_IDXGIFactory2_CreateSwapChainForHwnd,
> = None;
// static mut REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COREWINDOW: Option<
//     PFN_IDXGIFactory2_CreateSwapChainForCoreWindow,
// > = None;
// static mut REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_COMPOSITION: Option<
//     PFN_IDXGIFactory2_CreateSwapChainForComposition,
// > = None;
static mut REAL_CREATE_DXGI_FACTORY1: Option<PFN_CreateDXGIFactory1> = None;

// --- simple helpers --------------------------------------------

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
        pAdapter,
        DriverType,
        Software,
        Flags,
        pFeatureLevels,
        FeatureLevels,
        SDKVersion,
        ppDevice,
        pFeatureLevel,
        ppImmediateContext,
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

unsafe fn ensure_minhook_init() {
    if !MH_INIT_DONE.swap(true, Ordering::SeqCst) {
        if MH_Initialize() != 0 {
            dbg("MH_Initialize failed");
        }
        dbg("MH_Initialize ok");
    }
}

unsafe fn try_install_d3d_hooks_if_modules_present() {
    if HOOKS_INSTALLED_FOR_D3D.load(Ordering::SeqCst) {
        return;
    }

    let d3d11_opt = GetModuleHandleA(PCSTR(b"d3d11.dll\0".as_ptr())).ok();
    let dxgi_opt = GetModuleHandleA(PCSTR(b"dxgi.dll\0".as_ptr())).ok();
    if d3d11_opt.is_none() || dxgi_opt.is_none() {
        return;
    }

    let d3d11 = d3d11_opt.unwrap();
    let dxgi = dxgi_opt.unwrap();

    let addr_create_dev = GetProcAddress(d3d11, PCSTR(b"D3D11CreateDevice\0".as_ptr()));
    let addr_factory1 = GetProcAddress(dxgi, PCSTR(b"CreateDXGIFactory1\0".as_ptr()));

    let addr_create_dev = addr_create_dev
        .map(|f| f as *mut c_void)
        .unwrap_or(std::ptr::null_mut());
    let addr_factory1 = addr_factory1
        .map(|f| f as *mut c_void)
        .unwrap_or(std::ptr::null_mut());

    ensure_minhook_init();

    if !addr_create_dev.is_null() {
        if let Some(original_create_dev) =
            minhook_init_and_hook(addr_create_dev, my_d3d11_create_device as *mut _)
        {
            REAL_D3D11_CREATE_DEVICE = Some(std::mem::transmute::<
                *mut c_void,
                PFN_D3D11CreateDevice,
            >(original_create_dev));
            dbg("[hook] Hooked D3D11CreateDevice");
        }
    }
    if !addr_factory1.is_null() {
        if let Some(original_factory1) =
            minhook_init_and_hook(addr_factory1, my_create_dxgi_factory1 as *mut _)
        {
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

    // ---- 2) Try CreateDXGIFactory2 export to get a real IDXGIFactory2 (no QueryInterface)
    let dxgi = match GetModuleHandleA(PCSTR(b"dxgi.dll\0".as_ptr())).ok() {
        Some(h) => h,
        None => {
            dbg("[hook] dxgi.dll not loaded yet (unexpected here)");
            return;
        }
    };
    let pfn_cdf2 = GetProcAddress(dxgi, PCSTR(b"CreateDXGIFactory2\0".as_ptr()))
        .map(|f| f as *const ())
        .unwrap_or(std::ptr::null());

    if pfn_cdf2.is_null() {
        dbg("[hook] CreateDXGIFactory2 not exported on this system; skipping Factory2 hooks");
        return;
    }

    let create_dxgi_factory2: PFN_CreateDXGIFactory2 = std::mem::transmute(pfn_cdf2);

    let mut pf2: *mut c_void = std::ptr::null_mut();
    // Flags=0 is fine; we just need a COM object
    let hr2 = create_dxgi_factory2(0, &IDXGIFactory2::IID, &mut pf2);
    if hr2.is_err() || pf2.is_null() {
        dbg("[hook] CreateDXGIFactory2 failed; skipping Factory2 hooks");
        return;
    }

    let f2_iface: IDXGIFactory2 = transmute_copy(&pf2);
    let f2_ptr: *mut IDXGIFactory2 = transmute_copy(&f2_iface);

    let addr_for_hwnd = vtbl_entry(f2_ptr, 15);

    if !addr_for_hwnd.is_null() {
        if let Some(orig) = minhook_init_and_hook(
            addr_for_hwnd,
            my_factory2_create_swap_chain_for_hwnd as *mut _,
        ) {
            REAL_IDXGIFACTORY2_CREATE_SWAPCHAIN_FOR_HWND = Some(std::mem::transmute(orig));
            dbg("[hook] globally hooked IDXGIFactory2::CreateSwapChainForHwnd");
        }
    }
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
    unsafe {
        try_install_d3d_hooks_if_modules_present();
    }
    h
}

#[no_mangle]
pub extern "system" fn my_LoadLibraryW(name: PCWSTR) -> *mut c_void {
    let real = unsafe { REAL_LOADLIB_W.expect("REAL_LOADLIB_W") };
    let h = real(name);
    unsafe {
        try_install_d3d_hooks_if_modules_present();
    }
    h
}

#[ctor::ctor]
fn init() {
    unsafe {
        ensure_minhook_init();

        // Resolve kernel32 LoadLibrary exports.
        let k32 = GetModuleHandleA(PCSTR(b"kernel32.dll\0".as_ptr())).ok();
        if let Some(k32) = k32 {
            let addr_ll_a = GetProcAddress(k32, PCSTR(b"LoadLibraryA\0".as_ptr()))
                .map(|f| f as *mut c_void)
                .unwrap_or(std::ptr::null_mut());
            let addr_ll_w = GetProcAddress(k32, PCSTR(b"LoadLibraryW\0".as_ptr()))
                .map(|f| f as *mut c_void)
                .unwrap_or(std::ptr::null_mut());

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

            let pipe_name = r"\\.\pipe\masher_overlay_v2.0.2-beta";
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

                    let connected = ConnectNamedPipe(handle, ptr::null_mut());
                    if connected.as_bool() == false {
                        let err = GetLastError();
                        if err != ERROR_PIPE_CONNECTED {
                            dbg(&format!("ConnectNamedPipe failed: {}", err.0));
                            thread::sleep(Duration::from_secs(1));
                            continue;
                        }
                    }

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
                            }
                            "masher_inactive" => {
                                MASHER_ACTIVE.store(false, Ordering::SeqCst);
                            }
                            other => dbg("Unknown command: {other}"),
                        }
                    }

                    FlushFileBuffers(handle);
                    DisconnectNamedPipe(handle);
                    // Loop back to accept next client

                    dbg("Client disconnected!");
                }
            }
        });
    });
}
