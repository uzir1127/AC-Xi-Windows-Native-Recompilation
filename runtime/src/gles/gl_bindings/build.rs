/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use gl_generator::{Api, Fallbacks, GlobalGenerator, Profile, Registry};
use std::fs::File;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let mut file = File::create(out_dir.join("gles11.rs")).unwrap();
    Registry::new(
        Api::Gles1,
        (1, 1),
        Profile::Core,
        Fallbacks::None,
        [
            "GL_OES_framebuffer_object",
            "GL_OES_rgb8_rgba8",
            "GL_EXT_texture_filter_anisotropic",
            "GL_IMG_texture_compression_pvrtc",
            "GL_EXT_texture_lod_bias",
            "GL_EXT_texture_format_BGRA8888",
            "GL_OES_draw_texture",
            "GL_OES_mapbuffer",
            // Part of the OpenGL ES 1.1 common profile.
            "GL_OES_compressed_paletted_texture",
            "GL_OES_matrix_palette",
            "GL_OES_point_sprite",
            "GL_OES_blend_subtract",
        ],
    )
    .write_bindings(GlobalGenerator, &mut file)
    .unwrap();

    // Instrument only the raw ANGLE boundary, without counting guest ABI
    // conversion or texture decompression as ANGLE's own work. Normal builds
    // generate exactly the original bindings and contain no profiler calls.
    if std::env::var_os("CARGO_FEATURE_PROFILE_ANGLE").is_some() {
        drop(file);
        let path = out_dir.join("gles11.rs");
        let source = std::fs::read_to_string(&path).unwrap();
        let mut result = String::new();
        let mut rest = source.as_str();
        let mut id = 0;
        while let Some(pos) = rest.find("pub unsafe fn ") {
            let body = rest[pos..].find('{').unwrap() + pos;
            let name_end = rest[pos..].find('(').unwrap() + pos;
            let name = &rest[pos + "pub unsafe fn ".len()..name_end];
            result.push_str(&rest[..body + 1]);
            result.push_str(&format!(" let _profile_scope = crate::profile::Scope::enter({id}, \"gl{name}\"); "));
            rest = &rest[body + 1..];
            id += 1;
        }
        assert!(id > 100 && id < 400);
        result.push_str(rest);
        std::fs::write(path, result).unwrap();
    }
}
