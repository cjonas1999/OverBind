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

// ---------- caches for real resolvers ----------
static REAL_EGL_GETPROC: OnceCell<Sym> = OnceCell::new();
static REAL_GLX_GETPROC: OnceCell<Sym> = OnceCell::new();
static REAL_GLX_GETPROC_ARB: OnceCell<Sym> = OnceCell::new();

// ---------- caches for original targets (what our proxies will call) ----------
static ORIG_GLCLEAR: OnceCell<Sym> = OnceCell::new();
static ORIG_GLDRAWELEMENTS: OnceCell<Sym> = OnceCell::new();
static ORIG_GLDRAWARRAYS: OnceCell<Sym> = OnceCell::new();
static ORIG_EGL_GET_CURRENT_CONTEXT: OnceCell<Sym> = OnceCell::new();
static ORIG_EGL_GET_CURRENT_DISPLAY: OnceCell<Sym> = OnceCell::new();
static ORIG_EGL_GET_CURRENT_SURFACE: OnceCell<Sym> = OnceCell::new();
static ORIG_EGL_QUERY_SURFACE: OnceCell<Sym> = OnceCell::new();
// add any GL funcs you need in render path:
static ORIG_GL_GETINTEGERV: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_VIEWPORT: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_BINDFRAMEBUFFER: OnceCell<Sym> = OnceCell::new();
static ORIG_GLX_GET_CURRENT_CONTEXT: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_DISABLE: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_ENABLE: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_BLEND_FUNC: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_BLEND_EQUATION: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_USEPROGRAM: OnceCell<Sym> = OnceCell::new();
static ORIG_GL_CLEARCOLOR: OnceCell<Sym> = OnceCell::new();

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

// ---------- resolve helper: try NEXT then DEFAULT ----------
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

// ---------- get real resolver functions (do NOT recurse) ----------
unsafe fn real_egl_getproc() -> Option<extern "C" fn(*const c_char) -> *const c_void> {
    let sym = REAL_EGL_GETPROC.get_or_init(|| Sym(resolve_any(b"eglGetProcAddress\0")));
    if sym.0.is_null() {
        None
    } else {
        Some(mem::transmute(sym.0))
    }
}
unsafe fn real_glx_getproc() -> Option<extern "C" fn(*const u8) -> *const c_void> {
    let sym = REAL_GLX_GETPROC.get_or_init(|| Sym(resolve_any(b"glXGetProcAddress\0")));
    if sym.0.is_null() {
        None
    } else {
        Some(mem::transmute(sym.0))
    }
}
unsafe fn real_glx_getproc_arb() -> Option<extern "C" fn(*const u8) -> *const c_void> {
    let sym = REAL_GLX_GETPROC_ARB.get_or_init(|| Sym(resolve_any(b"glXGetProcAddressARB\0")));
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
unsafe fn c_name_c_char_to_str(name: *const c_char) -> String {
    if name.is_null() {
        return String::new();
    }
    CStr::from_ptr(name).to_string_lossy().into_owned()
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

// ---------- hook eglGetProcAddress ----------
#[no_mangle]
pub unsafe extern "C" fn eglGetProcAddress(name: *const c_char) -> *const c_void {
    // Get the real function via RTLD_NEXT
    let real: extern "C" fn(*const libc::c_char) -> *const c_void =
        std::mem::transmute(libc::dlsym(
            libc::RTLD_NEXT,
            b"eglGetProcAddress\0".as_ptr() as *const i8,
        ));

    // Cache the original for later
    let _ = REAL_EGL_GETPROC.set(Sym(real as *mut c_void));

    // Bypass interception if we’re inside a proxy
    if IN_PROXY.with(|c| c.get()) {
        return real(name);
    }

    // Otherwise, normal interception logic
    let want = CStr::from_ptr(name).to_string_lossy();
    let orig = real(name);
    if orig.is_null() {
        return orig;
    }

    match want.as_ref() {
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
        "eglGetCurrentContext" => {
            let _ = ORIG_EGL_GET_CURRENT_CONTEXT.set(Sym(orig as *mut c_void));
            return orig;
        }
        "eglGetCurrentDisplay" => {
            let _ = ORIG_EGL_GET_CURRENT_DISPLAY.set(Sym(orig as *mut c_void));
            return orig;
        }
        "eglGetCurrentSurface" => {
            let _ = ORIG_EGL_GET_CURRENT_SURFACE.set(Sym(orig as *mut c_void));
            return orig;
        }
        "eglQuerySurface" => {
            let _ = ORIG_EGL_QUERY_SURFACE.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glGetIntegerv" => {
            let _ = ORIG_GL_GETINTEGERV.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glViewport" => {
            let _ = ORIG_GL_VIEWPORT.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glBindFramebuffer" => {
            let _ = ORIG_GL_BINDFRAMEBUFFER.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glXGetCurrentContext" => {
            let _ = ORIG_GLX_GET_CURRENT_CONTEXT.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glDisable" => {
            let _ = ORIG_GL_DISABLE.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glEnable" => {
            let _ = ORIG_GL_ENABLE.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glBlendFunc" => {
            let _ = ORIG_GL_BLEND_FUNC.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glBlendEquation" => {
            let _ = ORIG_GL_BLEND_EQUATION.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glUseProgram" => {
            let _ = ORIG_GL_USEPROGRAM.set(Sym(orig as *mut c_void));
            return orig;
        }
        "glClearColor" => {
            let _ = ORIG_GL_CLEARCOLOR.set(Sym(orig as *mut c_void));
            return orig;
        }
        _ => orig,
    }
}

// ---------- hook glXGetProcAddress ----------
#[no_mangle]
pub unsafe extern "C" fn glXGetProcAddress(name: *const u8) -> *const c_void {
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

// ---------- hook glXGetProcAddressARB ----------
#[no_mangle]
pub unsafe extern "C" fn glXGetProcAddressARB(name: *const u8) -> *const c_void {
    let real = match real_glx_getproc_arb() {
        Some(f) => f,
        None => {
            log("real glXGetProcAddressARB not found");
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

// -----------------------------------------------------------------------------
// NOTE:
// - `render_imgui_for_current_context()` must be defined elsewhere in your crate.
// - Add additional symbol names to the `match` arms as needed (e.g., "glEnable",
//   "glDisable", "glBindBuffer", etc.) if the program obtains those pointers and
//   you want to intercept them.
// - Keep the proxies small and fast: they run in the game process and mustn't panic.
// - Make sure to build as `cdylib` and LD_PRELOAD your .so as before.
// -----------------------------------------------------------------------------

// Simple frame counter so we don't spam logs
static FRAME_COUNT: AtomicUsize = AtomicUsize::new(0);

//
// Utility: resolve symbol from RTLD_NEXT and cache
//
unsafe fn resolve_symbol(name: &CStr) -> *mut c_void {
    let sym = libc::dlsym(RTLD_NEXT, name.as_ptr());
    if sym.is_null() {
        log(&format!("dlsym failed for {}", name.to_string_lossy()));
    }
    sym
}

type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GlXGetProcAddress = unsafe extern "C" fn(*const u8) -> *const c_void; // GLubyte = u8

static EGL_GET_PROC: OnceCell<Sym> = OnceCell::new();
static GLX_GET_PROC: OnceCell<Sym> = OnceCell::new();
static GLX_GET_PROC_ARB: OnceCell<Sym> = OnceCell::new();

// Try to resolve eglGetProcAddress/glXGetProcAddress(ARB) once and cache them
unsafe fn get_egl_get_proc() -> Option<EglGetProcAddress> {
    let sym = EGL_GET_PROC.get_or_init(|| {
        let n = CStr::from_bytes_with_nul_unchecked(b"eglGetProcAddress\0");
        Sym(resolve_symbol(n))
    });
    if sym.0.is_null() {
        None
    } else {
        Some(mem::transmute(sym.0))
    }
}

unsafe fn get_glx_get_proc() -> Option<GlXGetProcAddress> {
    let sym = GLX_GET_PROC.get_or_init(|| {
        let n = CStr::from_bytes_with_nul_unchecked(b"glXGetProcAddress\0");
        Sym(resolve_symbol(n))
    });
    if sym.0.is_null() {
        None
    } else {
        Some(mem::transmute(sym.0))
    }
}

unsafe fn get_glx_get_proc_arb() -> Option<GlXGetProcAddress> {
    let sym = GLX_GET_PROC_ARB.get_or_init(|| {
        let n = CStr::from_bytes_with_nul_unchecked(b"glXGetProcAddressARB\0");
        Sym(resolve_symbol(n))
    });
    if sym.0.is_null() {
        None
    } else {
        Some(mem::transmute(sym.0))
    }
}

/// Cross-API loader used by imgui_opengl_renderer::Renderer::new
/// Tries (in order): eglGetProcAddress, glXGetProcAddress, glXGetProcAddressARB, finally dlsym(RTLD_NEXT, name).
unsafe fn gl_proc_loader(name: &str) -> *const c_void {
    // C string for egl/dlsym
    let c_name = match CString::new(name) {
        Ok(s) => s,
        Err(_) => return ptr::null(),
    };

    // 1) eglGetProcAddress(const char*)
    if let Some(egl_get) = get_egl_get_proc() {
        log(&format!("eglGetProcAddress called for {}", name));
        let p = egl_get(c_name.as_ptr());
        if !p.is_null() {
            return p;
        }
    }

    // 2) glXGetProcAddress / glXGetProcAddressARB(const GLubyte*)
    let name_bytes = name.as_bytes();
    if let Some(glx_get) = get_glx_get_proc() {
        log(&format!("glXGetProcAddress called for {}", name));
        let p = glx_get(name_bytes.as_ptr());
        if !p.is_null() {
            return p;
        }
    }
    if let Some(glx_get_arb) = get_glx_get_proc_arb() {
        let p = glx_get_arb(name_bytes.as_ptr());
        if !p.is_null() {
            return p;
        }
    }

    // 3) Fallback: dlsym(RTLD_NEXT, name) — core symbols sometimes exported this way
    libc::dlsym(RTLD_NEXT, c_name.as_ptr()) as *const c_void
}

// unsafe fn egl_context_is_current() -> bool {
//     // Resolve eglGetCurrentContext via your loader
//     log("checking for current EGL context");
//     let sym = gl_proc_loader("eglGetCurrentContext");
//     if sym.is_null() {
//         return false;
//     }
//     type EglGetCurrentContext = extern "C" fn() -> *mut c_void;
//     let f: EglGetCurrentContext = std::mem::transmute(sym);
//     !f().is_null()
// }

unsafe fn context_is_current() -> bool {
    log("checking for current context");
    return true;
    // First try EGL
    if let Some(sym) = ORIG_EGL_GET_CURRENT_CONTEXT.get() {
        let f: extern "C" fn() -> *mut c_void = std::mem::transmute(sym.0);
        if !f().is_null() {
            return true;
        }
    }
    // Fallback to GLX
    if let Some(sym) = ORIG_GLX_GET_CURRENT_CONTEXT.get() {
        let f: extern "C" fn() -> *mut c_void = std::mem::transmute(sym.0);
        if !f().is_null() {
            return true;
        }
    }
    false
}

struct Overlay {
    // the OpenGL renderer from imgui-opengl-renderer
    renderer: Option<imgui_opengl_renderer::Renderer>,
    show_demo: bool,
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
            ov_opt.replace(Overlay {
                renderer: None,
                show_demo: true,
            });
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
    // Check that an EGL context is bound
    if !context_is_current() {
        return;
    }

    // Increment frame counter
    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

    // Log every ~60 frames so the file doesn't explode
    if frame >= 0 {
        log(&format!(
            "render_imgui_for_current_context called, frame {}",
            frame
        ));
    }

    with_overlay(|ctx, ov| {
        log("calling overlay callback");
        // lazy-create renderer when a valid GL context is current
        if ov.renderer.is_none() {
            log("creating renderer");
            if !try_init_renderer_if_ready(ov, ctx) {
                return;
            }
        }

        // let gl_clear_color: extern "C" fn(f32, f32, f32, f32) =
        //     std::mem::transmute(renderer_gl_loader("glClearColor"));
        // let gl_clear: extern "C" fn(u32) = std::mem::transmute(renderer_gl_loader("glClear"));

        // // GL_COLOR_BUFFER_BIT = 0x00004000
        // gl_clear_color(1.0, 0.0, 0.0, 1.0);
        // gl_clear(0x00004000);

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

        // log("restoring state");
        // guard.restore();
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

struct GlStateGuard {
    program: i32,
    array_buffer: i32,
    element_array_buffer: i32,
    vertex_array: i32,
    blend: bool,
    blend_src_rgb: i32,
    blend_dst_rgb: i32,
    blend_src_alpha: i32,
    blend_dst_alpha: i32,
    blend_eq_rgb: i32,
    blend_eq_alpha: i32,
    depth_test: bool,
    cull_face: bool,
    scissor_test: bool,
    active_tex: i32,
    texture_binding_2d: i32,
    viewport: [i32; 4],
}

impl GlStateGuard {
    unsafe fn save() -> Self {
        type GlGetIntegerv = unsafe extern "C" fn(u32, *mut i32);
        type GlIsEnabled = unsafe extern "C" fn(u32) -> u8;
        type GlGetBooleanv = unsafe extern "C" fn(u32, *mut u8);
        const GL_CURRENT_PROGRAM: u32 = 0x8B8D;
        const GL_ARRAY_BUFFER_BINDING: u32 = 0x8894;
        const GL_ELEMENT_ARRAY_BUFFER_BINDING: u32 = 0x8895;
        const GL_VERTEX_ARRAY_BINDING: u32 = 0x85B5;
        const GL_BLEND: u32 = 0x0BE2;
        const GL_BLEND_SRC_RGB: u32 = 0x80C9;
        const GL_BLEND_DST_RGB: u32 = 0x80C8;
        const GL_BLEND_SRC_ALPHA: u32 = 0x80CB;
        const GL_BLEND_DST_ALPHA: u32 = 0x80CA;
        const GL_BLEND_EQUATION_RGB: u32 = 0x8009;
        const GL_BLEND_EQUATION_ALPHA: u32 = 0x883D;
        const GL_DEPTH_TEST: u32 = 0x0B71;
        const GL_CULL_FACE: u32 = 0x0B44;
        const GL_SCISSOR_TEST: u32 = 0x0C11;
        const GL_ACTIVE_TEXTURE: u32 = 0x84E0;
        const GL_TEXTURE_BINDING_2D: u32 = 0x8069;
        const GL_VIEWPORT: u32 = 0x0BA2;

        let get_i: GlGetIntegerv = std::mem::transmute(gl_proc_loader("glGetIntegerv"));
        let is_enabled: GlIsEnabled = std::mem::transmute(gl_proc_loader("glIsEnabled"));

        let mut s = GlStateGuard {
            program: 0,
            array_buffer: 0,
            element_array_buffer: 0,
            vertex_array: 0,
            blend: false,
            blend_src_rgb: 0,
            blend_dst_rgb: 0,
            blend_src_alpha: 0,
            blend_dst_alpha: 0,
            blend_eq_rgb: 0,
            blend_eq_alpha: 0,
            depth_test: false,
            cull_face: false,
            scissor_test: false,
            active_tex: 0,
            texture_binding_2d: 0,
            viewport: [0; 4],
        };

        get_i(GL_CURRENT_PROGRAM, &mut s.program);
        get_i(GL_ARRAY_BUFFER_BINDING, &mut s.array_buffer);
        get_i(GL_ELEMENT_ARRAY_BUFFER_BINDING, &mut s.element_array_buffer);
        get_i(GL_VERTEX_ARRAY_BINDING, &mut s.vertex_array);
        s.blend = is_enabled(GL_BLEND) != 0;
        get_i(GL_BLEND_SRC_RGB, &mut s.blend_src_rgb);
        get_i(GL_BLEND_DST_RGB, &mut s.blend_dst_rgb);
        get_i(GL_BLEND_SRC_ALPHA, &mut s.blend_src_alpha);
        get_i(GL_BLEND_DST_ALPHA, &mut s.blend_dst_alpha);
        get_i(GL_BLEND_EQUATION_RGB, &mut s.blend_eq_rgb);
        get_i(GL_BLEND_EQUATION_ALPHA, &mut s.blend_eq_alpha);
        s.depth_test = is_enabled(GL_DEPTH_TEST) != 0;
        s.cull_face = is_enabled(GL_CULL_FACE) != 0;
        s.scissor_test = is_enabled(GL_SCISSOR_TEST) != 0;
        get_i(GL_ACTIVE_TEXTURE, &mut s.active_tex);
        get_i(GL_TEXTURE_BINDING_2D, &mut s.texture_binding_2d);
        get_i(GL_VIEWPORT, s.viewport.as_mut_ptr());

        s
    }

    unsafe fn restore(&self) {
        type GlUseProgram = unsafe extern "C" fn(u32);
        type GlBindBuffer = unsafe extern "C" fn(u32, u32);
        type GlBindVertexArray = unsafe extern "C" fn(u32);
        type GlEnable = unsafe extern "C" fn(u32);
        type GlDisable = unsafe extern "C" fn(u32);
        type GlBlendFuncSeparate = unsafe extern "C" fn(u32, u32, u32, u32);
        type GlBlendEquationSeparate = unsafe extern "C" fn(u32, u32);
        type GlActiveTexture = unsafe extern "C" fn(u32);
        type GlBindTexture = unsafe extern "C" fn(u32, u32);
        type GlViewport = unsafe extern "C" fn(i32, i32, i32, i32);

        const GL_ARRAY_BUFFER: u32 = 0x8892;
        const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
        const GL_BLEND: u32 = 0x0BE2;
        const GL_DEPTH_TEST: u32 = 0x0B71;
        const GL_CULL_FACE: u32 = 0x0B44;
        const GL_SCISSOR_TEST: u32 = 0x0C11;
        const GL_TEXTURE_2D: u32 = 0x0DE1;

        let use_program: GlUseProgram = std::mem::transmute(gl_proc_loader("glUseProgram"));
        let bind_buf: GlBindBuffer = std::mem::transmute(gl_proc_loader("glBindBuffer"));
        let bind_vao: GlBindVertexArray = std::mem::transmute(gl_proc_loader("glBindVertexArray"));
        let en: GlEnable = std::mem::transmute(gl_proc_loader("glEnable"));
        let dis: GlDisable = std::mem::transmute(gl_proc_loader("glDisable"));
        let blend_func_sep: GlBlendFuncSeparate =
            std::mem::transmute(gl_proc_loader("glBlendFuncSeparate"));
        let blend_eq_sep: GlBlendEquationSeparate =
            std::mem::transmute(gl_proc_loader("glBlendEquationSeparate"));
        let active_tex: GlActiveTexture = std::mem::transmute(gl_proc_loader("glActiveTexture"));
        let bind_tex: GlBindTexture = std::mem::transmute(gl_proc_loader("glBindTexture"));
        let viewport: GlViewport = std::mem::transmute(gl_proc_loader("glViewport"));

        use_program(self.program as u32);
        bind_buf(GL_ARRAY_BUFFER, self.array_buffer as u32);
        bind_buf(GL_ELEMENT_ARRAY_BUFFER, self.element_array_buffer as u32);
        bind_vao(self.vertex_array as u32);

        if self.blend {
            en(GL_BLEND)
        } else {
            dis(GL_BLEND)
        }
        if self.depth_test {
            en(GL_DEPTH_TEST)
        } else {
            dis(GL_DEPTH_TEST)
        }
        if self.cull_face {
            en(GL_CULL_FACE)
        } else {
            dis(GL_CULL_FACE)
        }
        if self.scissor_test {
            en(GL_SCISSOR_TEST)
        } else {
            dis(GL_SCISSOR_TEST)
        }

        blend_func_sep(
            self.blend_src_rgb as u32,
            self.blend_dst_rgb as u32,
            self.blend_src_alpha as u32,
            self.blend_dst_alpha as u32,
        );
        blend_eq_sep(self.blend_eq_rgb as u32, self.blend_eq_alpha as u32);

        active_tex(self.active_tex as u32);
        bind_tex(GL_TEXTURE_2D, self.texture_binding_2d as u32);

        viewport(
            self.viewport[0],
            self.viewport[1],
            self.viewport[2],
            self.viewport[3],
        );
    }
}
