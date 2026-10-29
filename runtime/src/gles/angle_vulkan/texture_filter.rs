// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Load-time texture parameters through a private ES3 share-group member.
//!
//! The bundled ANGLE GLES1 frontend advertises anisotropy but rejects its
//! setters. ES3 accepts the shared texture parameter. Keep its validation and
//! error state isolated from the guest, and never share with the compositor.

use super::*;

const EGL_DRAW: EglInt = 0x3059;
const EGL_READ: EglInt = 0x305a;
const EGL_CONTEXT_CLIENT_VERSION: EglInt = 0x3098;
const GL_EXTENSIONS: u32 = 0x1f03;
const GL_TEXTURE_2D: u32 = 0x0de1;
const GL_TEXTURE_MAX_ANISOTROPY_EXT: u32 = 0x84fe;
const GL_MAX_TEXTURE_MAX_ANISOTROPY_EXT: u32 = 0x84ff;

#[derive(Default)]
pub(super) struct State {
    attempted: bool,
    helper: Option<Helper>,
}

#[derive(Clone, Copy)]
struct Current {
    display: Handle,
    draw: Handle,
    read: Handle,
    context: Handle,
}
impl Current {
    fn capture(api: &Api) -> Self {
        unsafe {
            Self {
                display: (api.get_current_display)(),
                draw: (api.get_current_surface)(EGL_DRAW),
                read: (api.get_current_surface)(EGL_READ),
                context: (api.get_current_context)(),
            }
        }
    }
}

/// Every return path, including unwinding, restores the exact prior surfaces.
struct RestoreCurrent<'a> {
    owner: &'a AngleVulkanDisplay,
    previous: Current,
}
impl<'a> RestoreCurrent<'a> {
    fn switch(owner: &'a AngleVulkanDisplay, previous: Current, helper: Handle) -> Result<Self, String> {
        let restore = Self { owner, previous };
        if unsafe { (owner.api.make_current)(owner.display, previous.draw, previous.read, helper) } == 0 {
            // Capture the EGL error before the restoration call can replace it.
            return Err(owner.api.error("Activating texture-filter helper"));
        }
        Ok(restore)
    }
}
impl Drop for RestoreCurrent<'_> {
    fn drop(&mut self) {
        let p = self.previous;
        if unsafe { (self.owner.api.make_current)(p.display, p.draw, p.read, p.context) } == 0 {
            // Continuing would send guest calls to the helper's namespace and
            // state. A restoration failure must therefore be fatal.
            panic!("{}", self.owner.api.error("Restoring guest context after texture filtering"));
        }
    }
}

struct Gl {
    get_string: unsafe extern "system" fn(u32) -> *const u8,
    get_float: unsafe extern "system" fn(u32, *mut f32),
    get_tex_parameter: unsafe extern "system" fn(u32, u32, *mut f32),
    is_texture: unsafe extern "system" fn(u32) -> u8,
    bind_texture: unsafe extern "system" fn(u32, u32),
    tex_parameter: unsafe extern "system" fn(u32, u32, f32),
    get_error: unsafe extern "system" fn() -> u32,
    flush: unsafe extern "system" fn(),
}
impl Gl {
    fn load(owner: &AngleVulkanDisplay) -> Result<Self, String> {
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let ptr = owner.get_proc_address($name);
                if ptr.is_null() { return Err(concat!("missing ", $name).into()); }
                unsafe { std::mem::transmute::<*const c_void, $ty>(ptr) }
            }};
        }
        Ok(Self {
            get_string: load!("glGetString", unsafe extern "system" fn(u32) -> *const u8),
            get_float: load!("glGetFloatv", unsafe extern "system" fn(u32, *mut f32)),
            get_tex_parameter: load!("glGetTexParameterfv", unsafe extern "system" fn(u32, u32, *mut f32)),
            is_texture: load!("glIsTexture", unsafe extern "system" fn(u32) -> u8),
            bind_texture: load!("glBindTexture", unsafe extern "system" fn(u32, u32)),
            tex_parameter: load!("glTexParameterf", unsafe extern "system" fn(u32, u32, f32)),
            get_error: load!("glGetError", unsafe extern "system" fn() -> u32),
            flush: load!("glFlush", unsafe extern "system" fn()),
        })
    }

    /// Call only while the private helper is current. Never consume guest errors.
    unsafe fn check_error(&self, operation: &str) -> Result<(), String> {
        let error = (self.get_error)();
        if error == 0 { Ok(()) } else { Err(format!("{operation}: GLES3 error 0x{error:04x}")) }
    }

    unsafe fn maximum(&self) -> Result<f32, String> {
        let extensions = (self.get_string)(GL_EXTENSIONS);
        self.check_error("Reading texture-filter extensions")?;
        if extensions.is_null() || !CStr::from_ptr(extensions.cast()).to_bytes()
            .split(|c| c.is_ascii_whitespace()).any(|e| e == b"GL_EXT_texture_filter_anisotropic")
        {
            return Err("GLES3 anisotropic filtering unavailable".into());
        }
        let mut maximum = 1.0;
        (self.get_float)(GL_MAX_TEXTURE_MAX_ANISOTROPY_EXT, &mut maximum);
        self.check_error("Reading texture-filter limit")?;
        if !maximum.is_finite() || maximum <= 1.0 {
            return Err("No usable GLES3 anisotropy limit".into());
        }
        Ok(maximum)
    }

    unsafe fn set(&self, texture: u32, requested: f32) -> Result<f32, String> {
        let exists = (self.is_texture)(texture) != 0;
        self.check_error("Checking shared guest texture")?;
        if !exists { return Err("Guest texture is not visible in helper share group".into()); }
        (self.bind_texture)(GL_TEXTURE_2D, texture);
        let result = (|| {
            self.check_error("Binding shared guest texture")?;
            let mut previous = 1.0;
            (self.get_tex_parameter)(GL_TEXTURE_2D, GL_TEXTURE_MAX_ANISOTROPY_EXT, &mut previous);
            self.check_error("Reading original shared texture filter")?;
            (self.tex_parameter)(GL_TEXTURE_2D, GL_TEXTURE_MAX_ANISOTROPY_EXT, requested);
            let mut actual = 1.0;
            (self.get_tex_parameter)(GL_TEXTURE_2D, GL_TEXTURE_MAX_ANISOTROPY_EXT, &mut actual);
            let error = self.check_error("Setting shared texture filter");
            if error.is_err() || !actual.is_finite() || (actual - requested).abs() > 0.001 {
                // If verification fails, retain the texture's previous setting.
                (self.tex_parameter)(GL_TEXTURE_2D, GL_TEXTURE_MAX_ANISOTROPY_EXT, previous);
                let _ = self.check_error("Restoring original shared texture filter");
                return Err(error.err().unwrap_or_else(|| format!("Texture filter readback {actual} differs from request {requested}")));
            }
            Ok(actual)
        })();
        // Do not retain a guest texture after its name is deleted or reused.
        (self.bind_texture)(GL_TEXTURE_2D, 0);
        (self.flush)();
        let cleanup = self.check_error("Releasing shared guest texture");
        result.and_then(|actual| cleanup.map(|()| actual))
    }
}

struct Helper {
    owner: Rc<AngleVulkanDisplay>,
    context: Handle,
    gl: Gl,
    maximum: Option<f32>,
}
impl Helper {
    fn new(guest: &AngleVulkanContext) -> Result<Self, String> {
        let owner = &guest.owner;
        let gl = Gl::load(owner)?;
        // Use the same framebuffer configuration and explicitly share this
        // guest context. Never infer the share group from whichever is current.
        let context = unsafe {
            (owner.api.create_context)(owner.display, owner.config, guest.context,
                [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE].as_ptr())
        };
        if context.is_null() { return Err(owner.api.error("Creating shared GLES3 texture-filter helper")); }
        Ok(Self { owner: Rc::clone(owner), context, gl, maximum: None })
    }

    fn apply(&mut self, previous: Current, texture: u32, requested: f32) -> Result<f32, String> {
        let _restore = RestoreCurrent::switch(&self.owner, previous, self.context)?;
        let maximum = match self.maximum {
            Some(value) => value,
            None => {
                let value = unsafe { self.gl.maximum()? };
                self.maximum = Some(value);
                log!("Texture filtering: shared GLES3 helper, device maximum {}x AF", value);
                value
            }
        };
        unsafe { self.gl.set(texture, requested.min(maximum)) }
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        // RestoreCurrent guarantees this context is no longer current here.
        unsafe { (self.owner.api.destroy_context)(self.owner.display, self.context); }
    }
}

impl AngleVulkanContext {
    /// Apply AF only during a guest texture load. Original mode does no work.
    /// Failure disables this guest's helper; no guest GL errors are consumed.
    /// After success, the caller must rebind its unchanged texture binding to
    /// observe the shared parameter change in the GLES1 sampler state.
    pub fn set_texture_anisotropy(&self, texture: u32, requested: f32) -> Option<f32> {
        if texture == 0 || !requested.is_finite() || requested <= 1.0 { return None; }
        let previous = Current::capture(&self.owner.api);
        if previous.context != self.context || previous.display != self.owner.display
            || self.owner.is_compositor_context(self.context)
        {
            return None;
        }
        let mut state = self.texture_filter.borrow_mut();
        if !state.attempted {
            state.attempted = true;
            match Helper::new(self) {
                Ok(helper) => state.helper = Some(helper),
                Err(error) => { log!("Texture filtering unavailable; keeping original filtering: {error}"); }
            }
        }
        let helper = state.helper.as_mut()?;
        unsafe {
            // Submit any preceding upload without a CPU wait. The helper only
            // changes sampler state; it does not read or modify texture pixels.
            (helper.gl.flush)();
        }
        let result = helper.apply(previous, texture, requested);
        match result {
            Ok(actual) => Some(actual),
            Err(error) => {
                log!("Texture filtering disabled; keeping original filtering: {error}");
                state.helper = None;
                None
            }
        }
    }
}
