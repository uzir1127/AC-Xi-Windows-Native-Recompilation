/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Utilities for presenting frames to the window using an abstract OpenGL ES
//! implementation.

use super::gles11_raw as gles11; // constants and types only
use super::GLES;
use crate::matrix::Matrix;
use std::time::{Duration, Instant};

pub struct FpsCounter {
    time: std::time::Instant,
    frames: u32,
}
impl FpsCounter {
    pub fn start() -> Self {
        FpsCounter {
            time: Instant::now(),
            frames: 0,
        }
    }

    pub fn count_frame(&mut self, label: std::fmt::Arguments<'_>) {
        self.frames += 1;
        let now = Instant::now();
        let duration = now - self.time;
        if duration >= Duration::from_secs(1) {
            self.time = now;
            echo!(
                "touchHLE: {} FPS: {:.2}",
                label,
                std::mem::take(&mut self.frames) as f32 / duration.as_secs_f32()
            );
        }
    }
}

/// Present the the latest frame (e.g. the app's splash screen or rendering
/// output), provided as a texture bound to `GL_TEXTURE_2D`, by drawing it on
/// the window. It may be rotated, scaled and/or letterboxed as necessary. The
/// virtual cursor is also drawn if it should be currently visible.
///
/// The provided context must be current.
pub unsafe fn present_frame(
    gles: &mut dyn GLES,
    viewport: (u32, u32, u32, u32),
    rotation_matrix: Matrix<2>,
    virtual_cursor_visible_at: Option<(f32, f32, bool)>,
) {
    crate::performance::display_frame();
    // While this is a generic utility, it is closely tied to
    // crate::frameworks::opengles::eagl::present_renderbuffer, which handles
    // backing up and restoring OpenGL ES state that this function might touch,
    // so these need to be updated in tandem.

    use gles11::types::*;

    // Draw the quad
    gles.Viewport(
        viewport.0 as _,
        viewport.1 as _,
        viewport.2 as _,
        viewport.3 as _,
    );
    gles.ClearColor(0.0, 0.0, 0.0, 1.0);
    gles.Clear(gles11::COLOR_BUFFER_BIT | gles11::DEPTH_BUFFER_BIT | gles11::STENCIL_BUFFER_BIT);
    gles.BindBuffer(gles11::ARRAY_BUFFER, 0);
    let vertices: [f32; 12] = [
        -1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0,
    ];
    gles.EnableClientState(gles11::VERTEX_ARRAY);
    gles.VertexPointer(2, gles11::FLOAT, 0, vertices.as_ptr() as *const GLvoid);
    let tex_coords: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    gles.EnableClientState(gles11::TEXTURE_COORD_ARRAY);
    gles.TexCoordPointer(2, gles11::FLOAT, 0, tex_coords.as_ptr() as *const GLvoid);
    let matrix = Matrix::<4>::from(&rotation_matrix);
    gles.MatrixMode(gles11::TEXTURE);
    gles.LoadMatrixf(matrix.columns().as_ptr() as *const _);
    gles.Enable(gles11::TEXTURE_2D);
    gles.DrawArrays(gles11::TRIANGLES, 0, 6);
    // clean this up so we don't need to worry about it in e.g. Core Animation
    gles.LoadIdentity();

    // Display virtual cursor
    if let Some((x, y, pressed)) = virtual_cursor_visible_at {
        let (vx, vy, vw, vh) = viewport;
        let x = x - vx as f32;
        let y = y - vy as f32;

        gles.DisableClientState(gles11::TEXTURE_COORD_ARRAY);
        gles.Disable(gles11::TEXTURE_2D);

        gles.Enable(gles11::BLEND);
        gles.BlendFunc(gles11::ONE, gles11::ONE_MINUS_SRC_ALPHA);
        gles.Color4f(0.0, 0.0, 0.0, if pressed { 2.0 / 3.0 } else { 1.0 / 3.0 });

        let radius = 10.0;

        let mut vertices = vertices;
        for i in (0..vertices.len()).step_by(2) {
            vertices[i] = (vertices[i] * radius + x) / (vw as f32 / 2.0) - 1.0;
            vertices[i + 1] = 1.0 - (vertices[i + 1] * radius + y) / (vh as f32 / 2.0);
        }
        gles.VertexPointer(2, gles11::FLOAT, 0, vertices.as_ptr() as *const GLvoid);
        gles.DrawArrays(gles11::TRIANGLES, 0, 6);
    }

    // The desktop menu is composited by the same hardware Vulkan context.
    // EAGL's caller restores the guest graphics state after presentation.
    if let Some(pixels) = crate::desktop::overlay() {
        let mut texture=0;
        gles.GenTextures(1,&mut texture);
        gles.BindTexture(gles11::TEXTURE_2D,texture);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_MIN_FILTER,gles11::LINEAR as _);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_MAG_FILTER,gles11::LINEAR as _);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_WRAP_S,gles11::CLAMP_TO_EDGE as _);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_WRAP_T,gles11::CLAMP_TO_EDGE as _);
        gles.TexImage2D(gles11::TEXTURE_2D,0,gles11::RGBA as _,960,640,0,gles11::RGBA,gles11::UNSIGNED_BYTE,pixels.as_ptr().cast());
        let quad:[f32;12]=[-1.0,-1.0,-1.0,1.0,1.0,-1.0,1.0,-1.0,-1.0,1.0,1.0,1.0];
        let uv:[f32;12]=[0.0,1.0,0.0,0.0,1.0,1.0,1.0,1.0,0.0,0.0,1.0,0.0];
        gles.EnableClientState(gles11::VERTEX_ARRAY);gles.EnableClientState(gles11::TEXTURE_COORD_ARRAY);
        gles.DisableClientState(gles11::COLOR_ARRAY);
        gles.VertexPointer(2,gles11::FLOAT,0,quad.as_ptr().cast());
        gles.TexCoordPointer(2,gles11::FLOAT,0,uv.as_ptr().cast());
        gles.Enable(gles11::TEXTURE_2D);gles.Enable(gles11::BLEND);
        gles.BlendFunc(gles11::SRC_ALPHA,gles11::ONE_MINUS_SRC_ALPHA);
        gles.Color4f(1.0,1.0,1.0,1.0);
        gles.DrawArrays(gles11::TRIANGLES,0,6);
        gles.DeleteTextures(1,&texture);
    }
    if let Some(pixels) = crate::performance::overlay() {
        let (w,h)=(crate::performance::WIDTH,crate::performance::HEIGHT);
        // The game viewport is centered/aspect-fitted. Anchor host telemetry
        // to the output window, including its letterbox/pillarbox area.
        let output_w=viewport.2+viewport.0*2;
        let output_h=viewport.3+viewport.1*2;
        gles.Viewport(0,0,output_w as _,output_h as _);
        let scale=(output_h as f32/640.0).clamp(0.75,1.5)
            .min((output_w as f32-16.0).max(1.0)/w as f32);
        let margin=8.0*scale;
        let left=-1.0+2.0*margin/output_w as f32;
        let right=left+2.0*w as f32*scale/output_w as f32;
        let top=1.0-2.0*margin/output_h as f32;
        let bottom=top-2.0*h as f32*scale/output_h as f32;
        let quad:[f32;12]=[left,bottom,left,top,right,bottom,right,bottom,left,top,right,top];
        let uv:[f32;12]=[0.0,1.0,0.0,0.0,1.0,1.0,1.0,1.0,0.0,0.0,1.0,0.0];
        let mut texture=0;gles.GenTextures(1,&mut texture);gles.BindTexture(gles11::TEXTURE_2D,texture);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_MIN_FILTER,gles11::LINEAR as _);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_MAG_FILTER,gles11::LINEAR as _);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_WRAP_S,gles11::CLAMP_TO_EDGE as _);
        gles.TexParameteri(gles11::TEXTURE_2D,gles11::TEXTURE_WRAP_T,gles11::CLAMP_TO_EDGE as _);
        gles.TexImage2D(gles11::TEXTURE_2D,0,gles11::RGBA as _,w as _,h as _,0,gles11::RGBA,gles11::UNSIGNED_BYTE,pixels.as_ptr().cast());
        gles.EnableClientState(gles11::VERTEX_ARRAY);gles.EnableClientState(gles11::TEXTURE_COORD_ARRAY);
        gles.DisableClientState(gles11::COLOR_ARRAY);
        gles.VertexPointer(2,gles11::FLOAT,0,quad.as_ptr().cast());
        gles.TexCoordPointer(2,gles11::FLOAT,0,uv.as_ptr().cast());
        gles.Enable(gles11::TEXTURE_2D);gles.Enable(gles11::BLEND);
        gles.BlendFunc(gles11::SRC_ALPHA,gles11::ONE_MINUS_SRC_ALPHA);gles.Color4f(1.0,1.0,1.0,1.0);
        gles.DrawArrays(gles11::TRIANGLES,0,6);gles.DeleteTextures(1,&texture);
    }
}
