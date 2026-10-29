/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! GLES state used to isolate presentation/overlays from original game drawing.
//! These are interface constants, independent of any desktop OpenGL backend.

use super::gles11_raw as gles11;
use super::gles11_raw::types::GLenum;

/// Capabilities backed up and restored around the presentation quad.
pub const CAPABILITIES: &[GLenum] = &[
    gles11::ALPHA_TEST,
    gles11::BLEND,
    gles11::COLOR_LOGIC_OP,
    gles11::CLIP_PLANE0,
    gles11::CLIP_PLANE1,
    gles11::CLIP_PLANE2,
    gles11::CLIP_PLANE3,
    gles11::CLIP_PLANE4,
    gles11::CLIP_PLANE5,
    gles11::LIGHT0,
    gles11::LIGHT1,
    gles11::LIGHT2,
    gles11::LIGHT3,
    gles11::LIGHT4,
    gles11::LIGHT5,
    gles11::LIGHT6,
    gles11::LIGHT7,
    gles11::COLOR_MATERIAL,
    gles11::CULL_FACE,
    gles11::DEPTH_TEST,
    gles11::DITHER,
    gles11::FOG,
    gles11::LIGHTING,
    gles11::LINE_SMOOTH,
    gles11::MULTISAMPLE,
    gles11::NORMALIZE,
    gles11::POINT_SMOOTH,
    gles11::POLYGON_OFFSET_FILL,
    gles11::RESCALE_NORMAL,
    gles11::SAMPLE_ALPHA_TO_COVERAGE,
    gles11::SAMPLE_ALPHA_TO_ONE,
    gles11::SAMPLE_COVERAGE,
    gles11::SCISSOR_TEST,
    gles11::STENCIL_TEST,
    gles11::TEXTURE_2D,
    // Same as POINT_SPRITE_OES from the GLES extension
    gles11::POINT_SPRITE_OES,
];

pub struct ArrayInfo {
    pub name: GLenum,
    pub buffer_binding: GLenum,
    pub pointer: GLenum,
}

/// Client arrays used for presentation state restoration and pointer queries.
pub const ARRAYS: &[ArrayInfo] = &[
    ArrayInfo {
        name: gles11::COLOR_ARRAY,
        buffer_binding: gles11::COLOR_ARRAY_BUFFER_BINDING,
        pointer: gles11::COLOR_ARRAY_POINTER,
    },
    ArrayInfo {
        name: gles11::NORMAL_ARRAY,
        buffer_binding: gles11::NORMAL_ARRAY_BUFFER_BINDING,
        pointer: gles11::NORMAL_ARRAY_POINTER,
    },
    ArrayInfo {
        name: gles11::TEXTURE_COORD_ARRAY,
        buffer_binding: gles11::TEXTURE_COORD_ARRAY_BUFFER_BINDING,
        pointer: gles11::TEXTURE_COORD_ARRAY_POINTER,
    },
    ArrayInfo {
        name: gles11::VERTEX_ARRAY,
        buffer_binding: gles11::VERTEX_ARRAY_BUFFER_BINDING,
        pointer: gles11::VERTEX_ARRAY_POINTER,
    },
];
