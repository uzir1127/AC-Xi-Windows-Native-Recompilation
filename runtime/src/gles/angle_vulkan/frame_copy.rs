// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! GPU snapshots between independent guest and compositor contexts.
//!
//! EGLImage imports the guest renderbuffer without sharing object namespaces.
//! A GPU copy gives the compositor a stable snapshot, rather than a live alias
//! that the next guest frame could overwrite. Server fences order both sides.
use super::*;
use crate::gles::gles11_raw as gl;
use std::cell::RefCell;

#[cfg(feature = "profile-angle")]
mod tests;

type CreateImage = unsafe extern "system" fn(Handle, Handle, u32, Handle, *const i32) -> Handle;
type CreateSync = unsafe extern "system" fn(Handle, u32, *const i32) -> Handle;
type WaitSync = unsafe extern "system" fn(Handle, Handle, i32) -> i32;
type ImageTarget = unsafe extern "system" fn(u32, Handle);

struct Extensions {
    create_image: CreateImage,
    destroy_image: Destroy,
    create_sync: CreateSync,
    destroy_sync: Destroy,
    wait_sync: WaitSync,
    image_target: ImageTarget,
}

#[derive(Default)]
pub(super) struct State {
    compositor: Cell<Handle>,
    extensions: RefCell<Option<Extensions>>,
    checked: Cell<bool>,
    disabled: Cell<bool>,
    retired: RefCell<Vec<[u32; 2]>>,
    copies: Cell<u64>,
    #[cfg(feature = "profile-angle")]
    mode_check: Cell<Option<std::time::Instant>>,
    #[cfg(feature = "profile-angle")]
    force_cpu: Cell<bool>,
}

/// Owns only compositor GL objects. Layer animation clones retain this owner.
pub(crate) struct GpuFrame {
    owner: Rc<AngleVulkanDisplay>,
    objects: [u32; 2], // texture, framebuffer
    width: i32,
    height: i32,
    opaque: bool,
    allocated: Cell<bool>,
}
impl GpuFrame {
    pub(crate) fn texture(&self) -> u32 { self.objects[0] }
}
impl Drop for GpuFrame {
    fn drop(&mut self) {
        // A layer may be destroyed with a guest context current. Never delete
        // these numeric names there: they belong to a different namespace.
        // If there are no more snapshots, context destruction frees them.
        self.owner.frame_copy.retired.borrow_mut().push(self.objects);
    }
}

impl AngleVulkanDisplay {
    pub(super) fn is_compositor_context(&self, context: Handle) -> bool {
        self.frame_copy.compositor.get() == context
    }

    pub(crate) fn register_compositor_context(&self) {
        self.frame_copy.compositor.set(unsafe { (self.api.get_current_context)() });
    }

    fn check_frame_copy(&self) -> Result<(), String> {
        let s = &self.frame_copy;
        if s.checked.replace(true) { return Ok(()); }
        if std::env::var_os("ACXI_READBACK_PRESENT").is_some() {
            return Err("diagnostic ACXI_READBACK_PRESENT requested".into());
        }
        // Prefer GPU snapshots. The Windows NVIDIA swapchain workaround is
        // applied before ANGLE initialization in bin.rs. Keep explicit CPU
        // readback available above for driver diagnosis and extension fallback.
        type QueryString = unsafe extern "system" fn(Handle, i32) -> *const c_char;
        let ptr = self.get_proc_address("eglQueryString");
        if ptr.is_null() { return Err("eglQueryString unavailable".into()); }
        let query: QueryString = unsafe { std::mem::transmute(ptr) };
        let ptr = unsafe { query(self.display, 0x3055) };
        if ptr.is_null() { return Err("EGL extension list unavailable".into()); }
        let egl_ext = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
        for name in ["EGL_KHR_image_base", "EGL_KHR_gl_renderbuffer_image", "EGL_KHR_fence_sync", "EGL_KHR_wait_sync"] {
            if !egl_ext.split_whitespace().any(|e| e == name) { return Err(format!("missing {name}")); }
        }
        let ptr = unsafe { gl::GetString(gl::EXTENSIONS) };
        if ptr.is_null() { return Err("GLES extension list unavailable".into()); }
        let gl_ext = unsafe { CStr::from_ptr(ptr.cast()) }.to_string_lossy();
        for name in ["GL_OES_EGL_image", "GL_OES_EGL_sync"] {
            if !gl_ext.split_whitespace().any(|e| e == name) { return Err(format!("missing {name}")); }
        }
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let ptr = self.get_proc_address($name);
                if ptr.is_null() { return Err(concat!("missing ", $name).into()); }
                unsafe { std::mem::transmute::<*const c_void, $ty>(ptr) }
            }};
        }
        *s.extensions.borrow_mut() = Some(Extensions {
            create_image: load!("eglCreateImageKHR", CreateImage),
            destroy_image: load!("eglDestroyImageKHR", Destroy),
            create_sync: load!("eglCreateSyncKHR", CreateSync),
            destroy_sync: load!("eglDestroySyncKHR", Destroy),
            wait_sync: load!("eglWaitSyncKHR", WaitSync),
            image_target: load!("glEGLImageTargetRenderbufferStorageOES", ImageTarget),
        });
        Ok(())
    }

    fn switch_frame_context(&self, context: Handle) -> Result<(), String> {
        if unsafe { (self.api.make_current)(self.display, self.surface.get(), self.surface.get(), context) } == 0 {
            Err(self.api.error("Switching frame-copy context"))
        } else { Ok(()) }
    }

    pub(crate) fn snapshot_frame(self: &Rc<Self>, previous: Option<Rc<GpuFrame>>, opaque: bool) -> Option<Rc<GpuFrame>> {
        #[cfg(feature = "profile-angle")]
        let _profile = touchHLE_gl_bindings::profile::Scope::enter(404, "gpu_frame_snapshot");
        let s = &self.frame_copy;
        // Profiling-only same-scene A/B control. Absent from normal builds.
        #[cfg(feature = "profile-angle")]
        {
            static CONTROL: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
            if let Some(path) = CONTROL.get_or_init(|| std::env::var_os("ACXI_PRESENT_CONTROL").map(PathBuf::from)) {
                let now = std::time::Instant::now();
                if s.mode_check.get().is_none_or(|last| now.duration_since(last).as_secs() >= 1) {
                    s.mode_check.set(Some(now));
                    if let Ok(mode) = std::fs::read_to_string(path) {
                        if matches!(mode.trim(), "cpu" | "gpu") {
                            let cpu = mode.trim() == "cpu";
                            if s.force_cpu.replace(cpu) != cpu {
                                log!("Profiling presentation mode: {}", if cpu { "CPU readback" } else { "GPU snapshot" });
                            }
                        }
                    }
                }
                if s.force_cpu.get() { return None; }
            }
        }
        if s.disabled.get() { return None; }
        let result = self.check_frame_copy().and_then(|_| unsafe { self.copy_frame(previous, opaque) });
        match result {
            Ok(frame) => {
                let n = s.copies.get() + 1;
                s.copies.set(n);
                if n == 1 || n % 1800 == 0 {
                    log!("GPU frame snapshots: {n}; EGLImage + GPU copy, no CPU readback");
                }
                Some(frame)
            }
            Err(error) => {
                s.disabled.set(true);
                log!("GPU frame snapshot unavailable; retaining CPU presentation: {error}");
                None
            }
        }
    }

    unsafe fn copy_frame(self: &Rc<Self>, previous: Option<Rc<GpuFrame>>, opaque: bool) -> Result<Rc<GpuFrame>, String> {
        let s = &self.frame_copy;
        let extensions = s.extensions.borrow();
        let e = extensions.as_ref().ok_or("frame-copy extensions not loaded")?;
        let producer = (self.api.get_current_context)();
        let consumer = s.compositor.get();
        if producer.is_null() || consumer.is_null() || producer == consumer {
            return Err("expected distinct guest and compositor contexts".into());
        }
        let (mut renderbuffer, mut width, mut height) = (0, 0, 0);
        gl::GetIntegerv(gl::RENDERBUFFER_BINDING_OES, &mut renderbuffer);
        gl::GetRenderbufferParameterivOES(gl::RENDERBUFFER_OES, gl::RENDERBUFFER_WIDTH_OES, &mut width);
        gl::GetRenderbufferParameterivOES(gl::RENDERBUFFER_OES, gl::RENDERBUFFER_HEIGHT_OES, &mut height);
        if renderbuffer == 0 || width <= 0 || height <= 0 { return Err("invalid source renderbuffer".into()); }

        // Recreate the image wrapper each present, so storage replacement or
        // reuse of a deleted guest renderbuffer name can never leave a stale alias.
        // EGL_IMAGE_PRESERVED_KHR is essential: importing must keep guest pixels.
        let image = (e.create_image)(self.display, producer, 0x30b9, renderbuffer as usize as Handle, [0x30d2, 1, EGL_NONE].as_ptr());
        if image.is_null() { return Err(self.api.error("Exporting guest renderbuffer")); }
        let ready = (e.create_sync)(self.display, 0x30f9, [EGL_NONE].as_ptr());
        if ready.is_null() {
            (e.destroy_image)(self.display, image);
            return Err(self.api.error("Creating producer fence"));
        }
        gl::Flush(); // submit before another context waits; no CPU completion wait
        if let Err(error) = self.switch_frame_context(consumer) {
            (e.destroy_sync)(self.display, ready);
            (e.destroy_image)(self.display, image);
            self.switch_frame_context(producer).expect("Restore guest after failed frame copy");
            return Err(error);
        }

        let result = (|| {
            if (e.wait_sync)(self.display, ready, 0) == 0 { return Err(self.api.error("Waiting for rendered frame")); }
            for [texture, framebuffer] in s.retired.borrow_mut().drain(..) {
                gl::DeleteFramebuffersOES(1, &framebuffer);
                gl::DeleteTextures(1, &texture);
            }
            let frame = match previous {
                Some(frame) if frame.width == width && frame.height == height && frame.opaque == opaque => frame,
                _ => {
                    let mut objects = [0; 2];
                    gl::GenTextures(1, &mut objects[0]);
                    gl::GenFramebuffersOES(1, &mut objects[1]);
                    Rc::new(GpuFrame { owner: Rc::clone(self), objects, width, height, opaque, allocated: Cell::new(false) })
                }
            };
            let (mut old_fb, mut old_rb, mut old_tex) = (0, 0, 0);
            gl::GetIntegerv(gl::FRAMEBUFFER_BINDING_OES, &mut old_fb);
            gl::GetIntegerv(gl::RENDERBUFFER_BINDING_OES, &mut old_rb);
            gl::GetIntegerv(gl::TEXTURE_BINDING_2D, &mut old_tex);
            let mut imported_rb = 0;
            gl::GenRenderbuffersOES(1, &mut imported_rb);
            gl::BindRenderbufferOES(gl::RENDERBUFFER_OES, imported_rb);
            (e.image_target)(gl::RENDERBUFFER_OES, image);
            gl::BindFramebufferOES(gl::FRAMEBUFFER_OES, frame.objects[1]);
            gl::FramebufferRenderbufferOES(gl::FRAMEBUFFER_OES, gl::COLOR_ATTACHMENT0_OES, gl::RENDERBUFFER_OES, imported_rb);
            let complete = gl::CheckFramebufferStatusOES(gl::FRAMEBUFFER_OES) == gl::FRAMEBUFFER_COMPLETE_OES;
            gl::BindTexture(gl::TEXTURE_2D, frame.objects[0]);
            // Allocate on first use only. RGB discards source alpha for opaque
            // CAEAGLLayers, matching the old CPU path's alpha=255 conversion.
            // Allocation state is shared by all presentation-layer clones.
            if !frame.allocated.replace(true) {
                gl::TexImage2D(gl::TEXTURE_2D, 0, if opaque { gl::RGB } else { gl::RGBA } as i32, width, height, 0, if opaque { gl::RGB } else { gl::RGBA }, gl::UNSIGNED_BYTE, null_mut());
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
                // Default REPEAT wrapping matches the original compositor upload.
            }
            if complete { gl::CopyTexSubImage2D(gl::TEXTURE_2D, 0, 0, 0, 0, 0, width, height); }
            // Detach the imported sibling before destroying its EGLImage.
            gl::FramebufferRenderbufferOES(gl::FRAMEBUFFER_OES, gl::COLOR_ATTACHMENT0_OES, gl::RENDERBUFFER_OES, 0);
            // Destroy the sibling as well as the EGLImage handle; otherwise
            // ANGLE correctly rejects exporting this source again next frame.
            gl::DeleteRenderbuffersOES(1, &imported_rb);
            gl::BindFramebufferOES(gl::FRAMEBUFFER_OES, old_fb as u32);
            gl::BindRenderbufferOES(gl::RENDERBUFFER_OES, old_rb as u32);
            gl::BindTexture(gl::TEXTURE_2D, old_tex as u32);
            let error = gl::GetError(); // compositor errors only; never consume guest GL errors
            if !complete || error != 0 { return Err(format!("GPU snapshot framebuffer complete={complete}, GL error=0x{error:x}")); }
            Ok(frame)
        })();

        // Order the copy before the guest can overwrite its original buffer.
        // The copied texture and later composition are in the same context.
        let copied = (e.create_sync)(self.display, 0x30f9, [EGL_NONE].as_ptr());
        if copied.is_null() { gl::Finish(); } else { gl::Flush(); }
        (e.destroy_sync)(self.display, ready);
        (e.destroy_image)(self.display, image);
        self.switch_frame_context(producer).expect("Restore guest after GPU frame copy");
        if !copied.is_null() {
            let waited = (e.wait_sync)(self.display, copied, 0);
            if waited == 0 {
                // Failure recovery only: finish the consumer before any guest
                // writes. The steady-state path never calls glFinish.
                self.switch_frame_context(consumer).expect("Activate copy context for recovery");
                gl::Finish();
                self.switch_frame_context(producer).expect("Restore guest after copy recovery");
            }
            (e.destroy_sync)(self.display, copied);
            if waited == 0 { return Err("consumer fence wait failed".into()); }
        } else { return Err("consumer fence creation failed".into()); }
        result
    }
}
