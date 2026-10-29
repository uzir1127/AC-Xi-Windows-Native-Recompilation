// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Opt-in hardware integration check of the actual production snapshot path.
//! Runs only in a profiling build with ACXI_TEST_GPU_COPY set.
use super::*;

impl AngleVulkanDisplay {
    pub(crate) fn test_frame_copy(self: &Rc<Self>) {
        unsafe {
            let producer = self.create_context(false).expect("test guest context");
            let (mut rb, mut fb, mut tex) = (0, 0, 0);
            gl::GenRenderbuffersOES(1, &mut rb);
            gl::GenFramebuffersOES(1, &mut fb);
            gl::GenTextures(1, &mut tex);
            let mut frame = None;
            let mut compared = 0;
            for iteration in 0..96 {
                producer.make_current().unwrap();
                let (w, h) = if iteration % 8 < 4 { (7, 5) } else { (9, 3) };
                let opaque = iteration % 3 != 0;
                gl::BindRenderbufferOES(gl::RENDERBUFFER_OES, rb);
                // Cover repeated presents, same-size storage replacement, and resize.
                if iteration % 2 == 0 {
                    gl::RenderbufferStorageOES(gl::RENDERBUFFER_OES, gl::RGBA8_OES, w, h);
                }
                gl::BindFramebufferOES(gl::FRAMEBUFFER_OES, fb);
                gl::FramebufferRenderbufferOES(gl::FRAMEBUFFER_OES, gl::COLOR_ATTACHMENT0_OES, gl::RENDERBUFFER_OES, rb);
                assert_eq!(gl::CheckFramebufferStatusOES(gl::FRAMEBUFFER_OES), gl::FRAMEBUFFER_COMPLETE_OES);
                gl::ActiveTexture(gl::TEXTURE1);
                gl::BindTexture(gl::TEXTURE_2D, tex);
                gl::Viewport(1, 2, 3, 4);
                gl::Disable(gl::SCISSOR_TEST);
                gl::ClearColor(iteration as f32 / 96.0, 0.25, 0.75, 0.25);
                gl::Clear(gl::COLOR_BUFFER_BIT);
                gl::Enable(gl::SCISSOR_TEST);
                gl::Scissor(0, 0, w / 2, h / 2);
                gl::ClearColor(0.1, 0.9, 0.3, 0.5);
                gl::Clear(gl::COLOR_BUFFER_BIT);
                let mut reference = vec![0u8; (w * h * 4) as usize];
                gl::ReadPixels(0, 0, w, h, gl::RGBA, gl::UNSIGNED_BYTE, reference.as_mut_ptr().cast());
                if opaque { for pixel in reference.chunks_exact_mut(4) { pixel[3] = 255; } }
                assert_eq!(gl::GetError(), 0);

                frame = self.snapshot_frame(frame, opaque);
                let output = frame.as_ref().expect("GPU copy must succeed; fallback is a test failure");
                assert!(producer.is_current(), "copy changed guest context");
                for (name, expected) in [(gl::FRAMEBUFFER_BINDING_OES, fb), (gl::RENDERBUFFER_BINDING_OES, rb), (gl::TEXTURE_BINDING_2D, tex), (gl::ACTIVE_TEXTURE, gl::TEXTURE1)] {
                    let mut value = 0;
                    gl::GetIntegerv(name, &mut value);
                    assert_eq!(value as u32, expected, "guest binding changed: {name:x}");
                }
                let mut viewport = [0; 4];
                gl::GetIntegerv(gl::VIEWPORT, viewport.as_mut_ptr());
                assert_eq!(viewport, [1, 2, 3, 4]);
                assert_eq!(gl::IsEnabled(gl::SCISSOR_TEST), gl::TRUE);

                // Overwrite the original before reading the snapshot. A mere
                // imported alias would fail this comparison.
                gl::Disable(gl::SCISSOR_TEST);
                gl::ClearColor(1.0, 0.0, 1.0, 1.0);
                gl::Clear(gl::COLOR_BUFFER_BIT);
                gl::Finish(); // test observation only, never the production fast path
                self.switch_frame_context(self.frame_copy.compositor.get()).unwrap();
                let mut verify_fb = 0;
                gl::GenFramebuffersOES(1, &mut verify_fb);
                gl::BindFramebufferOES(gl::FRAMEBUFFER_OES, verify_fb);
                gl::FramebufferTexture2DOES(gl::FRAMEBUFFER_OES, gl::COLOR_ATTACHMENT0_OES, gl::TEXTURE_2D, output.texture(), 0);
                assert_eq!(gl::CheckFramebufferStatusOES(gl::FRAMEBUFFER_OES), gl::FRAMEBUFFER_COMPLETE_OES);
                let mut actual = vec![0u8; reference.len()];
                gl::ReadPixels(0, 0, w, h, gl::RGBA, gl::UNSIGNED_BYTE, actual.as_mut_ptr().cast());
                assert_eq!(gl::GetError(), 0);
                assert_eq!(actual, reference, "GPU/CPU pixels differ at iteration {iteration}, opaque={opaque}");
                compared += actual.len();
                gl::BindFramebufferOES(gl::FRAMEBUFFER_OES, 0);
                gl::DeleteFramebuffersOES(1, &verify_fb);
            }
            drop(frame);
            producer.make_current().unwrap();
            gl::DeleteTextures(1, &tex);
            gl::DeleteFramebuffersOES(1, &fb);
            gl::DeleteRenderbuffersOES(1, &rb);
            log!("GPU COPY TEST PASS: 96 exact pixel comparisons, {compared} bytes; repeated presents, opaque/alpha, orientation, resize, same-size storage replacement, guest state preservation, snapshot after source overwrite");
        }
    }
}
