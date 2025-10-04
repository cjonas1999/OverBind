use libc::{c_char, c_void, RTLD_DEFAULT, RTLD_NEXT};
use once_cell::sync::OnceCell;
use std::cell::RefCell;
use std::ffi::CStr;
use std::ffi::CString;
use std::io::Write;
use std::mem;
use std::ptr;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Once;

// ---------- small wrapper so OnceCell<T> can be Sync ----------
struct Sym(*mut c_void);
unsafe impl Send for Sym {}
unsafe impl Sync for Sym {}

// ---------- reentrancy guard (per thread) ----------
thread_local! { static IN_PROXY: std::cell::Cell<bool> = std::cell::Cell::new(false); }

fn enter_proxy() -> bool {
    let prev = IN_PROXY.with(|c| {
        let p = c.get();
        c.set(true);
        p
    });
    prev
}
fn leave_proxy() {
    IN_PROXY.with(|c| c.set(false));
}

// ---------- caches for real resolver ----------
static REAL_GLX_GETPROC: OnceCell<Sym> = OnceCell::new();

// ---------- caches for original targets (what our proxies will call) ----------
static ORIG_GLCLEAR: OnceCell<Sym> = OnceCell::new();
static ORIG_GLDRAWELEMENTS: OnceCell<Sym> = OnceCell::new();
static ORIG_GLDRAWARRAYS: OnceCell<Sym> = OnceCell::new();

static ORIG_EGL_GETPROCADDRESS: OnceCell<Sym> = OnceCell::new();

unsafe fn init_orig_egl_getprocaddress() {
    if ORIG_EGL_GETPROCADDRESS.get().is_none() {
        // Try common names
        let handle = libc::dlopen(b"libEGL.so.1\0".as_ptr() as *const i8, libc::RTLD_LAZY);
        if handle.is_null() {
            log("dlopen libEGL.so.1 failed, trying libEGL.so");
        }
        let handle = if handle.is_null() {
            libc::dlopen(b"libEGL.so\0".as_ptr() as *const i8, libc::RTLD_LAZY)
        } else {
            handle
        };

        if handle.is_null() {
            log("Failed to dlopen any libEGL.so");
            return;
        }

        let sym = libc::dlsym(handle, b"eglGetProcAddress\0".as_ptr() as *const i8);
        if !sym.is_null() {
            let _ = ORIG_EGL_GETPROCADDRESS.set(Sym(sym));
            log("Successfully grabbed eglGetProcAddress via dlopen(libEGL)");
        } else {
            log("dlsym on libEGL.so for eglGetProcAddress still failed");
        }
    }
}

unsafe fn resolve_any(name: &[u8]) -> *mut c_void {
    let cs = CStr::from_bytes_with_nul_unchecked(name);
    let mut p = libc::dlsym(RTLD_NEXT, cs.as_ptr());
    if p.is_null() {
        p = libc::dlsym(RTLD_DEFAULT, cs.as_ptr());
    }
    if p.is_null() {
        log(&format!("resolve_any failed: {}", cs.to_string_lossy()));
    }
    p
}

unsafe fn real_glx_getproc() -> Option<extern "C" fn(*const u8) -> *const c_void> {
    let sym = REAL_GLX_GETPROC.get_or_init(|| Sym(resolve_any(b"glXGetProcAddress\0")));
    if sym.0.is_null() {
        None
    } else {
        Some(mem::transmute(sym.0))
    }
}

// ---------- tiny helpers ----------
unsafe fn c_name_u8_to_str(name: *const u8) -> &'static str {
    let mut len = 0usize;
    while !name.is_null() && *name.add(len) != 0 {
        len += 1;
    }
    let bytes = std::slice::from_raw_parts(name, len);
    std::str::from_utf8_unchecked(bytes)
}
// ---------- interception table ----------
fn should_intercept(name: &str) -> bool {
    matches!(name, "glClear" | "glDrawElements" | "glDrawArrays")
}

// ---------- proxies (call overlay then original) ----------
unsafe extern "C" fn glClear_proxy(mask: u32) {
    init_orig_egl_getprocaddress();
    if enter_proxy() {
        // prevent re-entrancy
        if let Some(sym) = ORIG_GLCLEAR.get() {
            let orig: extern "C" fn(u32) = mem::transmute(sym.0);
            orig(mask);
        }
        return;
    }
    log("glClear_proxy");
    // draw overlay (your function should guard context etc.)
    render_imgui_for_current_context();
    log("finished rendering from glClear_proxy");
    // call original
    if let Some(sym) = ORIG_GLCLEAR.get() {
        if !sym.0.is_null() {
            let orig: extern "C" fn(u32) = mem::transmute(sym.0);
            orig(mask);
        }
    }
    leave_proxy();
}

unsafe extern "C" fn glDrawElements_proxy(mode: u32, count: i32, ty: u32, indices: *const c_void) {
    if enter_proxy() {
        if let Some(sym) = ORIG_GLDRAWELEMENTS.get() {
            let orig: extern "C" fn(u32, i32, u32, *const c_void) = mem::transmute(sym.0);
            orig(mode, count, ty, indices);
        }
        return;
    }
    log("glDrawElements_proxy");
    render_imgui_for_current_context();
    log("finished rendering from glDrawElements_proxy");
    if let Some(sym) = ORIG_GLDRAWELEMENTS.get() {
        if !sym.0.is_null() {
            let orig: extern "C" fn(u32, i32, u32, *const c_void) = mem::transmute(sym.0);
            orig(mode, count, ty, indices);
        }
    }
    leave_proxy();
}

unsafe extern "C" fn glDrawArrays_proxy(mode: u32, first: i32, count: i32) {
    if enter_proxy() {
        if let Some(sym) = ORIG_GLDRAWARRAYS.get() {
            let orig: extern "C" fn(u32, i32, i32) = mem::transmute(sym.0);
            orig(mode, first, count);
        }
        return;
    }
    log("glDrawArrays_proxy");
    render_imgui_for_current_context();
    log("finished rendering from glDrawArrays_proxy");
    if let Some(sym) = ORIG_GLDRAWARRAYS.get() {
        if !sym.0.is_null() {
            let orig: extern "C" fn(u32, i32, i32) = mem::transmute(sym.0);
            orig(mode, first, count);
        }
    }
    leave_proxy();
}

// ---------- hook glXGetProcAddress ----------
#[no_mangle]
pub unsafe extern "C" fn glXGetProcAddress(name: *const u8) -> *const c_void {
    log("Calling glXGetProcAddress");
    let real = match real_glx_getproc() {
        Some(f) => f,
        None => {
            log("real glXGetProcAddress not found");
            return ptr::null();
        }
    };
    let want = c_name_u8_to_str(name);
    let orig = real(name); // resolve original first

    if !should_intercept(want) || orig.is_null() {
        return orig;
    }

    match want {
        "glClear" => {
            let _ = ORIG_GLCLEAR.set(Sym(orig as *mut c_void));
            glClear_proxy as *const c_void
        }
        "glDrawElements" => {
            let _ = ORIG_GLDRAWELEMENTS.set(Sym(orig as *mut c_void));
            glDrawElements_proxy as *const c_void
        }
        "glDrawArrays" => {
            let _ = ORIG_GLDRAWARRAYS.set(Sym(orig as *mut c_void));
            glDrawArrays_proxy as *const c_void
        }
        _ => orig,
    }
}

struct Overlay {
    // the OpenGL renderer from imgui-opengl-renderer
    renderer: Option<imgui_opengl_renderer::Renderer>,
}
thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = RefCell::new(None);
}

static mut GLOBAL_CTX: Option<NonNull<imgui::Context>> = None;
static INIT: Once = Once::new();

fn get_imgui_context() -> &'static mut imgui::Context {
    unsafe {
        INIT.call_once(|| {
            let mut ctx = imgui::Context::create();
            ctx.set_ini_filename(None);
            log("Created global ImGui context");

            let fonts = ctx.fonts();

            let font_cfg = imgui::FontConfig {
                size_pixels: 64.0,
                ..Default::default()
            };

            let font_data = include_bytes!("../icons/IconFont.ttf");
            fonts.add_font(&[imgui::FontSource::TtfData {
                data: font_data,
                size_pixels: 64.0,
                config: Some(font_cfg),
            }]);
            ctx.fonts().build_rgba32_texture();

            GLOBAL_CTX = Some(NonNull::new_unchecked(Box::leak(Box::new(ctx))));
        });
        GLOBAL_CTX.unwrap().as_mut()
    }
}

fn with_overlay<F: FnOnce(&mut imgui::Context, &mut Overlay)>(f: F) {
    let ctx = get_imgui_context();
    OVERLAY.with(|cell| {
        log("fetched OVERLAY");
        let mut ov_opt = cell.borrow_mut();
        log("borrow_mut");
        if ov_opt.is_none() {
            log("replacing Overlay");
            ov_opt.replace(Overlay { renderer: None });
        }
        log("calling f");
        let ov = ov_opt.as_mut().expect("overlay state just set");

        f(ctx, ov);
    });
    log("with_overlay done");
}

unsafe fn renderer_gl_loader(name: &str) -> *const c_void {
    let cname = CString::new(name).unwrap();

    // Use the real eglGetProcAddress we captured
    if let Some(sym) = ORIG_EGL_GETPROCADDRESS.get() {
        let egl_get: extern "C" fn(*const i8) -> *const c_void = std::mem::transmute(sym.0);
        let ptr = egl_get(cname.as_ptr());
        if !ptr.is_null() {
            return ptr;
        }
    }

    // Fallback for core functions
    let ptr = libc::dlsym(libc::RTLD_DEFAULT, cname.as_ptr());
    if !ptr.is_null() {
        return ptr as *const c_void;
    }

    std::ptr::null()
}

unsafe fn try_init_renderer_if_ready(ov: &mut Overlay, ctx: &mut imgui::Context) -> bool {
    if ov.renderer.is_some() {
        return true;
    }

    // Safe to create renderer now
    ov.renderer = Some(imgui_opengl_renderer::Renderer::new(ctx, |s| {
        renderer_gl_loader(s)
    }));

    log("Renderer created successfully");
    true
}

/// Very basic stub: just logs that we were called.
pub unsafe fn render_imgui_for_current_context() {
    with_overlay(|ctx, ov| {
        log("calling overlay callback");
        // lazy-create renderer when a valid GL context is current
        if ov.renderer.is_none() {
            log("creating renderer");
            if !try_init_renderer_if_ready(ov, ctx) {
                return;
            }
        }

        // Resolve glGetIntegerv
        let gl_get: extern "C" fn(u32, *mut i32) =
            std::mem::transmute(renderer_gl_loader("glGetIntegerv"));

        // 4 ints: x, y, width, height
        let mut vp = [0i32; 4];
        gl_get(0x0BA2, vp.as_mut_ptr()); // GL_VIEWPORT = 0x0BA2

        let width = vp[2].max(1) as f32;
        let height = vp[3].max(1) as f32;
        log(&format!("viewport = {:?}", vp));
        ctx.io_mut().display_size = [width, height];

        let ui = ctx.new_frame();
        log("got new frame");

        {
            // Get the foreground draw list (always rendered on top)
            let draw_list = ui.get_foreground_draw_list();

            // Pick a font (optional — default font will work if it contains the glyph)
            draw_list.add_text(
                [20.0, 60.0],
                imgui::ImColor32::from_rgba(0x99, 0x99, 0x99, 0x99),
                " ",
            );
        }

        if let Some(renderer) = ov.renderer.as_mut() {
            log("rendering with renderer");
            renderer.render(ctx);
            log("finished rendering");
        }
    });
}

fn log(msg: &str) {
    return;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/masher.log")
        .and_then(|mut f| writeln!(f, "{}", msg))
        .ok();
}
