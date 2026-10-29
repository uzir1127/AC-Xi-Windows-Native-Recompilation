/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use cargo_license::{get_dependencies_from_cargo_lock, GetDependenciesOpt};
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.to_str().unwrap());
}

pub fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Generate a list of dependencies with license and author information.
    // This is used in license.rs

    let deps = get_dependencies_from_cargo_lock(
        Default::default(),
        GetDependenciesOpt {
            // The goal is to get a list of dependencies which are used in the
            // final binary, so we can comply with license terms for binary
            // distribution. Therefore, dev- and build-time dependencies don't
            // matter.
            avoid_dev_deps: true,
            avoid_build_deps: true,
            direct_deps_only: false,
            root_only: false,
        },
    )
    .unwrap();
    let mut deps_string = String::new();
    for dep in deps {
        // Exclude internal packages, they all use the same license and are
        // handled specially.
        if dep.name.starts_with("touchHLE") {
            continue;
        }

        write!(&mut deps_string, "- {} version {}", dep.name, dep.version).unwrap();
        if let Some(authors) = dep.authors {
            let authors: Vec<&str> = authors.split('|').collect();
            write!(&mut deps_string, " by {}", authors.join(", ")).unwrap();
        } else {
            write!(&mut deps_string, " (author unspecified)").unwrap();
        }
        if let Some(license) = dep.license {
            write!(&mut deps_string, ", licensed under {license}").unwrap();
        } else {
            panic!("Dependency {} has an unspecified license!", dep.name);
        }
        writeln!(&mut deps_string).unwrap();
    }

    std::fs::write(out_dir.join("rust_dependencies.txt"), deps_string).unwrap();

    rerun_if_changed(&package_root.join("Cargo.lock"));

    // Original MP4 movie playback: software Media Foundation decoding to
    // CPU RGBA/PCM. It creates no Direct3D device or presentation path.
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let movie_source = package_root.join("src/frameworks/media_player/native/acxi_movie.c");
        let movie_header = package_root.join("src/frameworks/media_player/native/acxi_movie.h");
        rerun_if_changed(&movie_source);
        rerun_if_changed(&movie_header);
        cc::Build::new().file(&movie_source).warnings(true).compile("acxi_movie");
        for library in ["mfplat", "mfreadwrite", "mfuuid", "ole32", "propsys", "uuid"] {
            println!("cargo:rustc-link-lib={library}");
        }
        let hardware_source = package_root.join("src/native/acxi_hardware.cpp");
        rerun_if_changed(&hardware_source);
        rerun_if_changed(&package_root.join("src/native/acxi_hardware.h"));
        cc::Build::new().cpp(true).std("c++17").file(hardware_source)
            .include(package_root.join("vendor/SDL/include"))
            .include(package_root.join("vendor/SDL/src/video/khronos"))
            .warnings(true).compile("acxi_hardware");
        println!("cargo:rustc-link-lib=psapi");
    }

    // ACXi native fork: Dynarmic is not linked. The AOT wrapper builds the
    // generated game translation independently; no Dynarmic submodule or
    // Boost headers are needed for this executable.
    println!("cargo:rustc-check-cfg=cfg(acxi_softfloat)");
    println!("cargo:rerun-if-env-changed=ACXI_AOT_GENERATED");
    let aot_generated = std::env::var_os("ACXI_AOT_GENERATED")
        .map(PathBuf::from)
        .unwrap_or_else(|| package_root.join("../local/generated"));
    let fp_source = aot_generated.join("acxi_softfloat.c");
    rerun_if_changed(&fp_source);
    if fp_source.is_file() {
        println!("cargo:rustc-cfg=acxi_softfloat");
    }

    // libc++_shared.so has to be copied into the APK. See README of cargo-ndk.
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "android" {
        println!("cargo:rustc-link-lib=c++_shared");
        let sysroot_libs_path =
            PathBuf::from(std::env::var_os("CARGO_NDK_SYSROOT_LIBS_PATH").unwrap());
        let lib_path = sysroot_libs_path.join("libc++_shared.so");
        std::fs::copy(
            lib_path,
            // cargo-ndk as invoked by cargo-ndk-android-gradle actually
            // copies from the target directory, using this hacky path
            // concatenation approach. :(
            package_root
                .join("target")
                .join(std::env::var("TARGET").unwrap())
                .join(std::env::var("PROFILE").unwrap())
                .join("libc++_shared.so"),
        )
        .unwrap();
    }

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        // Rust removed link to advapi32 here https://github.com/rust-lang/rust/pull/138233
        // but sdl2 still depends on it
        println!("cargo::rustc-link-lib=advapi32")
    }
}
