/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Shared utilities.

use super::gles11_raw as gles11; // constants only
use super::gles11_raw::types::{GLenum, GLint, GLsizei};
use super::GLES;

/// Helper for implementing `glCompressedTexImage2D`: if `internalformat` is
/// one of the `IMG_texture_compression_pvrtc` formats, decode it and call
/// `glTexImage2D`. Returns `true` if this is done.
///
/// Note that this panics rather than create GL errors for invalid use (TODO?)
#[allow(clippy::too_many_arguments)]
pub fn try_decode_pvrtc(
    gles: &mut dyn GLES,
    target: GLenum,
    level: GLint,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
    pvrtc_data: &[u8],
) -> bool {
    let is_2bit = match internalformat {
        gles11::COMPRESSED_RGB_PVRTC_4BPPV1_IMG | gles11::COMPRESSED_RGBA_PVRTC_4BPPV1_IMG => false,
        gles11::COMPRESSED_RGB_PVRTC_2BPPV1_IMG | gles11::COMPRESSED_RGBA_PVRTC_2BPPV1_IMG => true,
        _ => return false,
    };

    assert!(border == 0);
    let pixels = crate::image::decode_pvrtc(
        pvrtc_data,
        is_2bit,
        width.try_into().unwrap(),
        height.try_into().unwrap(),
    );
    unsafe {
        gles.TexImage2D(
            target,
            level,
            gles11::RGBA as _,
            width,
            height,
            border,
            gles11::RGBA,
            gles11::UNSIGNED_BYTE,
            pixels.as_ptr() as *const _,
        )
    };
    true
}

/// Marker for the ten standard paletted formats. Their decoding is handled
/// by ANGLE; the removed desktop GL backend needed additional format fields.
pub struct PalettedTextureFormat;
impl PalettedTextureFormat {
    pub fn get_info(internalformat: GLenum) -> Option<Self> {
        match internalformat {
            gles11::PALETTE4_RGB8_OES
            | gles11::PALETTE4_RGBA8_OES
            | gles11::PALETTE4_R5_G6_B5_OES
            | gles11::PALETTE4_RGBA4_OES
            | gles11::PALETTE4_RGB5_A1_OES
            | gles11::PALETTE8_RGB8_OES
            | gles11::PALETTE8_RGBA8_OES
            | gles11::PALETTE8_R5_G6_B5_OES
            | gles11::PALETTE8_RGBA4_OES
            | gles11::PALETTE8_RGB5_A1_OES => Some(Self),
            _ => None,
        }
    }
}
