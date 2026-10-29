/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Windows EGL host for ANGLE's GLES 1 implementation on hardware Vulkan.
//! The backend is selected explicitly; OpenGL, Direct3D and software fallback
//! are never requested. The guest API remains GLES 1, while GPU work is Vulkan.

use std::ffi::{c_char, c_void, CStr, CString};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::rc::Rc;

mod frame_copy;
mod texture_filter;
pub(crate) use frame_copy::GpuFrame;

type Handle = *mut c_void;
type EglBool = u32;
type EglInt = i32;

const EGL_NONE: EglInt = 0x3038;
const EGL_PLATFORM_ANGLE_ANGLE: u32 = 0x3202;
const EGL_PLATFORM_ANGLE_TYPE_ANGLE: EglInt = 0x3203;
const EGL_PLATFORM_ANGLE_TYPE_VULKAN_ANGLE: EglInt = 0x3450;
const EGL_PLATFORM_ANGLE_DEVICE_TYPE_ANGLE: EglInt = 0x3209;
const EGL_PLATFORM_ANGLE_DEVICE_TYPE_HARDWARE_ANGLE: EglInt = 0x320a;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const c_char) -> *const c_void;
    fn FreeLibrary(module: Handle) -> i32;
}

struct Library(Handle);
impl Library {
    fn open(path: &Path) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // An absolute path plus LOAD_WITH_ALTERED_SEARCH_PATH resolves ANGLE's
        // adjacent runtime dependencies, without changing process search paths.
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), null_mut(), 0x8) };
        if module.is_null() {
            Err(format!("Could not load {}: {}", path.display(), std::io::Error::last_os_error()))
        } else {
            Ok(Self(module))
        }
    }
    fn symbol(&self, name: &'static [u8]) -> Result<*const c_void, String> {
        let ptr = unsafe { GetProcAddress(self.0, name.as_ptr().cast()) };
        if ptr.is_null() {
            Err(format!("ANGLE is missing {}", String::from_utf8_lossy(name)))
        } else {
            Ok(ptr)
        }
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0); }
    }
}

type GetProc = unsafe extern "system" fn(*const c_char) -> *const c_void;
type GetPlatformDisplay = unsafe extern "system" fn(u32, Handle, *const EglInt) -> Handle;
type Initialize = unsafe extern "system" fn(Handle, *mut EglInt, *mut EglInt) -> EglBool;
type BindApi = unsafe extern "system" fn(u32) -> EglBool;
type ChooseConfig = unsafe extern "system" fn(Handle, *const EglInt, *mut Handle, EglInt, *mut EglInt) -> EglBool;
type CreateWindowSurface = unsafe extern "system" fn(Handle, Handle, Handle, *const EglInt) -> Handle;
type CreateContext = unsafe extern "system" fn(Handle, Handle, Handle, *const EglInt) -> Handle;
type MakeCurrent = unsafe extern "system" fn(Handle, Handle, Handle, Handle) -> EglBool;
type GetCurrentContext = unsafe extern "system" fn() -> Handle;
type GetCurrentSurface = unsafe extern "system" fn(EglInt) -> Handle;
type SwapBuffers = unsafe extern "system" fn(Handle, Handle) -> EglBool;
type SwapInterval = unsafe extern "system" fn(Handle, EglInt) -> EglBool;
type Destroy = unsafe extern "system" fn(Handle, Handle) -> EglBool;
type Terminate = unsafe extern "system" fn(Handle) -> EglBool;
type GetError = unsafe extern "system" fn() -> EglInt;
type QuerySurface = unsafe extern "system" fn(Handle, Handle, EglInt, *mut EglInt) -> EglBool;

// EGL_KHR_debug exposes ANGLE's underlying driver error, which eglGetError
// otherwise reduces to the unhelpful EGL_CONTEXT_LOST code.
unsafe extern "system" fn egl_debug_message(
    error: u32, command: *const c_char, _kind: EglInt,
    _thread: Handle, _object: Handle, message: *const c_char,
) {
    let command = if command.is_null() { std::borrow::Cow::Borrowed("unknown") }
        else { CStr::from_ptr(command).to_string_lossy() };
    let message = if message.is_null() { std::borrow::Cow::Borrowed("") }
        else { CStr::from_ptr(message).to_string_lossy() };
    let _ = writeln!(std::io::stderr(), "[ANGLE EGL {error:#x}] {command}: {message}");
}

struct Api {
    get_proc: GetProc,
    create_context: CreateContext,
    make_current: MakeCurrent,
    get_current_context: GetCurrentContext,
    get_current_display: GetCurrentContext,
    get_current_surface: GetCurrentSurface,
    swap_buffers: SwapBuffers,
    swap_interval: SwapInterval,
    destroy_context: Destroy,
    destroy_surface: Destroy,
    terminate: Terminate,
    get_error: GetError,
    query_surface: QuerySurface,
    // Libraries outlive every function pointer above.
    _egl: Library,
    _gles: Library,
}

pub struct AngleVulkanDisplay {
    frame_copy: frame_copy::State,
    api: Api,
    display: Handle,
    config: Handle,
    surface: Cell<Handle>,
    presentations: Cell<u64>,
    capture: Option<(u64, PathBuf)>,
    capture_every: Option<u64>,
}

pub struct AngleVulkanContext {
    owner: Rc<AngleVulkanDisplay>,
    context: Handle,
    texture_filter: RefCell<texture_filter::State>,
}

impl AngleVulkanDisplay {
    pub fn new(native_window: Handle) -> Result<Rc<Self>, String> {
        let folder: PathBuf = std::env::var_os("ACXI_ANGLE_DIR").map(PathBuf::from).unwrap_or_else(|| {
            std::env::current_exe().expect("No executable path").parent().unwrap().to_path_buf()
        });
        let folder = folder.canonicalize().map_err(|err| format!("ANGLE directory {}: {err}", folder.display()))?;
        let gles = Library::open(&folder.join("libGLESv2.dll"))?;
        let egl = Library::open(&folder.join("libEGL.dll"))?;
        // These signatures are the EGL 1.4 ABI from Khronos' egl.h.
        macro_rules! load {
            ($name:literal, $ty:ty) => {
                unsafe { std::mem::transmute::<*const c_void, $ty>(egl.symbol(concat!($name, "\0").as_bytes())?) }
            };
        }
        let get_proc = load!("eglGetProcAddress", GetProc);
        let debug_ptr = unsafe { get_proc(c"eglDebugMessageControlKHR".as_ptr()) };
        if !debug_ptr.is_null() {
            type DebugCallback = unsafe extern "system" fn(u32, *const c_char, EglInt, Handle, Handle, *const c_char);
            type DebugControl = unsafe extern "system" fn(DebugCallback, *const isize) -> EglInt;
            let debug: DebugControl = unsafe { std::mem::transmute(debug_ptr) };
            // Critical, error and warning messages; no per-frame info logging.
            unsafe { debug(egl_debug_message, [0x33b9, 1, 0x33ba, 1, 0x33bb, 1, 0x33bc, 0, EGL_NONE as isize].as_ptr()); }
        }
        let platform_ptr = unsafe { get_proc(c"eglGetPlatformDisplayEXT".as_ptr()) };
        if platform_ptr.is_null() { return Err("ANGLE lacks eglGetPlatformDisplayEXT".into()); }
        let get_platform: GetPlatformDisplay = unsafe { std::mem::transmute(platform_ptr) };
        let initialize = load!("eglInitialize", Initialize);
        let bind_api = load!("eglBindAPI", BindApi);
        let choose_config = load!("eglChooseConfig", ChooseConfig);
        let create_surface = load!("eglCreateWindowSurface", CreateWindowSurface);
        let api = Api {
            get_proc,
            create_context: load!("eglCreateContext", CreateContext),
            make_current: load!("eglMakeCurrent", MakeCurrent),
            get_current_context: load!("eglGetCurrentContext", GetCurrentContext),
            get_current_display: load!("eglGetCurrentDisplay", GetCurrentContext),
            get_current_surface: load!("eglGetCurrentSurface", GetCurrentSurface),
            swap_buffers: load!("eglSwapBuffers", SwapBuffers),
            swap_interval: load!("eglSwapInterval", SwapInterval),
            destroy_context: load!("eglDestroyContext", Destroy),
            destroy_surface: load!("eglDestroySurface", Destroy),
            terminate: load!("eglTerminate", Terminate),
            get_error: load!("eglGetError", GetError),
            query_surface: load!("eglQuerySurface", QuerySurface),
            _egl: egl,
            _gles: gles,
        };
        let attrs = [EGL_PLATFORM_ANGLE_TYPE_ANGLE, EGL_PLATFORM_ANGLE_TYPE_VULKAN_ANGLE,
            EGL_PLATFORM_ANGLE_DEVICE_TYPE_ANGLE, EGL_PLATFORM_ANGLE_DEVICE_TYPE_HARDWARE_ANGLE, EGL_NONE];
        let display = unsafe { get_platform(EGL_PLATFORM_ANGLE_ANGLE, null_mut(), attrs.as_ptr()) };
        if display.is_null() { return Err(api.error("Selecting hardware Vulkan")); }
        let (mut major, mut minor) = (0, 0);
        if unsafe { initialize(display, &mut major, &mut minor) } == 0 {
            return Err(api.error("Initializing hardware Vulkan"));
        }
        if unsafe { bind_api(0x30a0) } == 0 {
            unsafe { (api.terminate)(display); }
            return Err(api.error("Binding the GLES frontend"));
        }
        // Window surface, GLES1, RGBA8, depth16, stencil8.
        let config_attrs = [0x3033, 4, 0x3040, 1, 0x3024, 8, 0x3023, 8, 0x3022, 8,
            0x3021, 8, 0x3025, 16, 0x3026, 8, EGL_NONE];
        let (mut config, mut count) = (null_mut(), 0);
        if unsafe { choose_config(display, config_attrs.as_ptr(), &mut config, 1, &mut count) } == 0 || count == 0 {
            unsafe { (api.terminate)(display); }
            return Err(api.error("Choosing a Vulkan GLES1 framebuffer"));
        }
        let surface = unsafe { create_surface(display, config, native_window, [EGL_NONE].as_ptr()) };
        if surface.is_null() {
            let error = api.error("Creating the Vulkan window surface");
            unsafe { (api.terminate)(display); }
            return Err(error);
        }
        log!("ACXi renderer: ANGLE hardware Vulkan, EGL {}.{} (no backend fallback)", major, minor);
        let capture = std::env::var_os("ACXI_CAPTURE_BMP").map(|path| {
            let frame = std::env::var("ACXI_CAPTURE_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
            (frame, PathBuf::from(path))
        });
        let capture_every = std::env::var("ACXI_CAPTURE_EVERY").ok()
            .and_then(|value| value.parse::<u64>().ok()).filter(|&value| value > 0);
        Ok(Rc::new(Self { frame_copy: frame_copy::State::default(), api, display, config, surface: Cell::new(surface), presentations: Cell::new(0), capture, capture_every }))
    }

    /// Keep the EGLSurface and all contexts alive across HWND changes. ANGLE
    /// detects the native extent and recreates its own Vulkan swapchain at
    /// presentation. Avoid replacing shared presentation objects while native
    /// resize events and graphics context switches are being processed.
    pub fn resize_window(&self, change: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        change()
    }

    pub fn create_context(self: &Rc<Self>, share_current: bool) -> Result<AngleVulkanContext, String> {
        let share = if share_current { unsafe { (self.api.get_current_context)() } } else { null_mut() };
        let context = unsafe { (self.api.create_context)(self.display, self.config, share, [0x3098, 1, EGL_NONE].as_ptr()) };
        if context.is_null() { return Err(self.api.error("Creating Vulkan GLES1 context")); }
        let result = AngleVulkanContext {
            owner: Rc::clone(self), context, texture_filter: RefCell::default(),
        };
        result.make_current()?;
        self.verify_renderer()?;
        unsafe { (self.api.swap_interval)(self.display, 1); }
        Ok(result)
    }

    pub fn get_proc_address(&self, name: &str) -> *const c_void {
        let name = CString::new(name).expect("Graphics function name contains NUL");
        unsafe { (self.api.get_proc)(name.as_ptr()) }
    }

    fn verify_renderer(&self) -> Result<(), String> {
        type GetString = unsafe extern "system" fn(u32) -> *const c_char;
        let address = self.get_proc_address("glGetString");
        if address.is_null() { return Err("ANGLE is missing glGetString".into()); }
        let get_string: GetString = unsafe { std::mem::transmute(address) };
        let renderer_ptr = unsafe { get_string(0x1f01) };
        if renderer_ptr.is_null() { return Err("ANGLE did not identify the GPU backend".into()); }
        let renderer = unsafe { CStr::from_ptr(renderer_ptr) }.to_string_lossy();
        #[cfg(feature = "profile-angle")]
        {
            let ext = unsafe { get_string(0x1f03) };
            if !ext.is_null() { log!("Profiling GLES extensions: {}", unsafe {CStr::from_ptr(ext)}.to_string_lossy()); }
        }
        if !renderer.contains("Vulkan") || renderer.contains("SwiftShader") || renderer.contains("llvmpipe") {
            return Err(format!("Refusing a non-hardware-Vulkan renderer: {renderer}"));
        }
        log!("Verified Vulkan adapter: {}", renderer);
        // EGL_ANGLE_device_vulkan exposes this display's real Vulkan device.
        // Identity queries do not create or submit work to another renderer.
        type Query = unsafe extern "system" fn(Handle,i32,*mut isize)->u32;
        let query_display=self.get_proc_address("eglQueryDisplayAttribEXT");
        let query_device=self.get_proc_address("eglQueryDeviceAttribEXT");
        let (mut physical,mut instance,mut get_proc)=(0isize,0isize,0isize);
        if !query_display.is_null()&&!query_device.is_null() {
            let display_query:Query=unsafe {std::mem::transmute(query_display)};
            let device_query:Query=unsafe {std::mem::transmute(query_device)};
            let mut device=0isize;
            if unsafe {display_query(self.display,0x322c,&mut device)}!=0&&device!=0 {
                unsafe {
                    device_query(device as Handle,0x34ab,&mut physical);
                    device_query(device as Handle,0x34a9,&mut instance);
                    device_query(device as Handle,0x34d1,&mut get_proc);
                }
            }
        }
        crate::performance::renderer(&renderer,physical as Handle,instance as Handle,get_proc as Handle);
        Ok(())
    }

    pub fn swap_buffers(&self) {
        let frame = self.presentations.get() + 1;
        self.presentations.set(frame);
        if let Some((capture_frame, path)) = &self.capture {
            if frame == *capture_frame || self.capture_every.is_some_and(|every| frame % every == 0) {
                match self.capture_bmp(path) {
                    Ok(()) => {
                        // Periodic diagnostics replace this one image. Keep
                        // disk use bounded during unattended startup tests.
                        log!("Captured Vulkan presentation {} to {}", frame, path.display());
                    }
                    Err(error) => { log!("Vulkan capture failed: {}", error); }
                }
            }
        }
        #[cfg(feature = "profile-angle")]
        let swap_scope = touchHLE_gl_bindings::profile::Scope::enter(401, "eglSwapBuffers");
        if unsafe { (self.api.swap_buffers)(self.display, self.surface.get()) } == 0 {
            panic!("{}", self.api.error("Presenting the Vulkan swapchain"));
        }
        #[cfg(feature = "profile-angle")]
        {
            drop(swap_scope);
            touchHLE_gl_bindings::profile::frame();
        }
        if frame == 1 || frame % 300 == 0 {
            log!("Vulkan swapchain presentations: {} (includes splash/UI frames)", frame);
        }
    }

    fn capture_bmp(&self, path: &Path) -> Result<(), String> {
        let (mut width, mut height) = (0i32, 0i32);
        unsafe {
            if (self.api.query_surface)(self.display, self.surface.get(), 0x3057, &mut width) == 0 ||
               (self.api.query_surface)(self.display, self.surface.get(), 0x3056, &mut height) == 0 {
                return Err(self.api.error("Querying Vulkan capture size"));
            }
        }
        if width <= 0 || height <= 0 { return Err("Vulkan surface has no drawable area".into()); }
        type GetInt = unsafe extern "system" fn(u32, *mut i32);
        type BindFbo = unsafe extern "system" fn(u32, u32);
        type ReadPixels = unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
        // These GLES1 functions are required by the host compositor too.
        let functions = [self.get_proc_address("glGetIntegerv"), self.get_proc_address("glBindFramebufferOES"), self.get_proc_address("glReadPixels")];
        if functions.iter().any(|function| function.is_null()) {
            return Err("ANGLE lacks an entry point needed for capture".into());
        }
        let get_int: GetInt = unsafe { std::mem::transmute(functions[0]) };
        let bind_fbo: BindFbo = unsafe { std::mem::transmute(functions[1]) };
        let read_pixels: ReadPixels = unsafe { std::mem::transmute(functions[2]) };
        let len = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or("Capture size overflow")?;
        let mut pixels = vec![0u8; len];
        let mut old_fbo = 0;
        unsafe {
            get_int(0x8ca6, &mut old_fbo);
            bind_fbo(0x8d40, 0);
            read_pixels(0, 0, width, height, 0x1908, 0x1401, pixels.as_mut_ptr().cast());
            bind_fbo(0x8d40, old_fbo as u32);
        }
        for pixel in pixels.chunks_exact_mut(4) { pixel.swap(0, 2); }
        let size: u32 = len.try_into().map_err(|_| "Capture exceeds BMP format size")?;
        let mut header = Vec::with_capacity(54);
        header.extend_from_slice(b"BM");
        header.extend_from_slice(&(54 + size).to_le_bytes());
        header.extend_from_slice(&[0u8; 4]);
        header.extend_from_slice(&54u32.to_le_bytes());
        header.extend_from_slice(&40u32.to_le_bytes());
        header.extend_from_slice(&width.to_le_bytes());
        header.extend_from_slice(&height.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&32u16.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&size.to_le_bytes());
        header.extend_from_slice(&[0u8; 16]);
        let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        file.write_all(&header).and_then(|_| file.write_all(&pixels)).map_err(|e| e.to_string())
    }
}

impl Api {
    fn error(&self, operation: &str) -> String {
        format!("{operation}: EGL error 0x{:04x}", unsafe { (self.get_error)() })
    }
}

impl AngleVulkanContext {
    pub fn is_current(&self) -> bool {
        #[cfg(feature = "profile-angle")]
        let _profile_scope = touchHLE_gl_bindings::profile::Scope::enter(403, "eglGetCurrentContext");
        unsafe { (self.owner.api.get_current_context)() == self.context }
    }
    pub fn make_current(&self) -> Result<(), String> {
        #[cfg(feature = "profile-angle")]
        let _profile_scope = touchHLE_gl_bindings::profile::Scope::enter(402, "eglMakeCurrent");
        let owner = &self.owner;
        if unsafe { (owner.api.make_current)(owner.display, owner.surface.get(), owner.surface.get(), self.context) } == 0 {
            Err(owner.api.error("Activating Vulkan GLES1 context"))
        } else { Ok(()) }
    }
}

impl Drop for AngleVulkanContext {
    fn drop(&mut self) {
        // Release the private ES3 share-group member before its guest owner.
        *self.texture_filter.get_mut() = texture_filter::State::default();
        unsafe {
            if self.is_current() {
                (self.owner.api.make_current)(self.owner.display, null_mut(), null_mut(), null_mut());
            }
            (self.owner.api.destroy_context)(self.owner.display, self.context);
        }
    }
}
impl Drop for AngleVulkanDisplay {
    fn drop(&mut self) {
        unsafe {
            (self.api.make_current)(self.display, null_mut(), null_mut(), null_mut());
            (self.api.destroy_surface)(self.display, self.surface.get());
            (self.api.terminate)(self.display);
        }
    }
}
