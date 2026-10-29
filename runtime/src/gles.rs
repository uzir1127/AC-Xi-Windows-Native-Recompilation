/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The original game's GLES 1.1 interface backed exclusively by ANGLE Vulkan.
//!
//! Original game calls, UIKit/movie composition and desktop overlays share
//! this context. ANGLE translates GLES into Vulkan; this is not a direct
//! Vulkan engine renderer. There is no alternate backend or fallback.

pub(crate) mod angle_vulkan;
pub mod gles1_native;
mod gles_generic;
pub mod present;
pub(crate) mod state;
mod util;

pub use touchHLE_gl_bindings::gles11 as gles11_raw;
use gles1_native::GLES1NativeContext;
pub use gles_generic::{GLESContext, GLES};
use crate::environment::Environment;

pub fn create_gles1_ctx(env: &mut Environment) -> Box<dyn GLESContext> {
    env.on_parent_stack_in_coroutine(|window, _options| {
        create_gles1_ctx_no_parent_stack(window)
    })
}

/// Create the sole renderer on the main stack, outside any game coroutine.
pub fn create_gles1_ctx_no_parent_stack(
    window: &mut crate::window::Window,
) -> Box<dyn GLESContext> {
    assert!(window.on_main_stack());
    log!("Creating the Vulkan renderer (GLES 1.1 guest interface):");
    log!("Using: {}", GLES1NativeContext::description());
    Box::new(GLES1NativeContext::new(window).expect("Couldn't create the hardware Vulkan renderer"))
}
