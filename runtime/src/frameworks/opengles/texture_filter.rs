// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Optional anisotropic filtering for compressed game textures.
//!
//! Called only after guest compressed level-zero uploads. The original main
//! HUD, text and menu atlases use uncompressed uploads and are left alone.
//! This does not alter min/mag filters, wrap modes or mip levels, and never
//! generates mipmaps. Most ACXi textures intentionally have no mip chain.

use crate::gles::{gles11_raw as gl, GLES};
use std::sync::Once;

/// The caller must have the guest's valid ES1 context current and its uploaded
/// texture bound. The host implementation handles capability checks and any
/// context switching without touching the guest's pending error state.
pub(super) unsafe fn apply(gles: &mut dyn GLES, target: u32, level: i32) {
    let requested = crate::desktop::active_graphics().1;
    // Original mode has no GL queries or parameter changes. Higher mip uploads
    // retain the texture object's setting, with no per-bind/per-frame work.
    if requested <= 1 || target != gl::TEXTURE_2D || level != 0 {
        return;
    }
    match gles.set_texture_anisotropy(target, requested as f32) {
        Some(active) => {
            static ACTIVE_LOG: Once = Once::new();
            ACTIVE_LOG.call_once(|| {
                log!("Compressed game textures: {}x anisotropic filtering through shared host context; original filters and mip levels retained", active);
            });
        }
        None => {
            static FALLBACK_LOG: Once = Once::new();
            FALLBACK_LOG.call_once(|| {
                log!("Compressed game textures: anisotropic filtering unavailable; retaining original filtering");
            });
        }
    }
}
