/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
// Allow the crate to have a non-snake-case name (touchHLE).
// This also allows items in the crate to have non-snake-case names.
#![allow(non_snake_case)]

use sha1::{Digest, Sha1};
use std::io::Read;
use std::path::Path;

fn validate_game(path: &Path) -> Result<(), String> {
    // The statically generated code is tied to this executable. Accepting a
    // different binary could silently combine incompatible code and data.
    let binary = if path.extension().is_some_and(|ext| ext == "ipa") {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut executable = archive.by_name("Payload/ACE-Xi.app/ACE-Xi")
            .map_err(|e| format!("ACXi executable missing from IPA: {e}"))?;
        let mut bytes = Vec::new();
        executable.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        bytes
    } else {
        std::fs::read(path.join("ACE-Xi")).map_err(|e| e.to_string())?
    };
    let hash = format!("{:x}", Sha1::digest(&binary));
    if hash != "12cb706225c81c2707b1f876268d0ea825b33ffe" {
        return Err(format!("This native build requires the inspected ACXi 1.1.3 executable. Found SHA-1 {hash}."));
    }
    Ok(())
}

fn main() -> Result<(), String> {
    // This ANGLE version's deferred swapchain allocation reproducibly crashes
    // the NVIDIA driver during window/fullscreen transitions. Eager allocation
    // preserves the renderer, image quality and presentation timing. Set this
    // before SDL/ANGLE or telemetry can start any worker threads.
    let mut disabled = std::env::var("ANGLE_FEATURE_OVERRIDES_DISABLED").unwrap_or_default();
    if !disabled.split(':').any(|name| name == "swapchainDeferredMemoryAllocation") {
        if !disabled.is_empty() { disabled.push(':'); }
        disabled.push_str("swapchainDeferredMemoryAllocation");
        std::env::set_var("ANGLE_FEATURE_OVERRIDES_DISABLED", disabled);
    }
    let mut args: Vec<String> = std::env::args().collect();
    if args.len()==1 {
        let executable=std::env::current_exe().map_err(|e|e.to_string())?;
        let folder=executable.parent().ok_or("Cannot locate the native executable directory")?;
        let app=folder.join("game/ACE-Xi.app");
        if app.is_dir() {
            std::env::set_current_dir(folder).map_err(|e|e.to_string())?;
            args.push(app.to_string_lossy().into_owned());
        }
    }
    if let Some(game) = args.iter().skip(1).find(|a| a.ends_with(".ipa") || a.ends_with(".app")) {
        validate_game(Path::new(game))?;
    } else if !args.iter().any(|a| matches!(a.as_str(), "--help" | "--copyright")) {
        return Err("Supply your ACXi 1.1.3 .ipa or .app path. This is a game-specific native build.".to_owned());
    }
    eprintln!("ACXi Native development build: static ARM-to-C execution / Vulkan-only renderer");
    touchHLE::main(args.into_iter())
}
