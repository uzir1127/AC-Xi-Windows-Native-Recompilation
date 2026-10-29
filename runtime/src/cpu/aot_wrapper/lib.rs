/*
 * ACXi native execution bridge, 2026-10-27.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
#![allow(non_snake_case)]

/// Exact C layout shared with generated/acxi_aot.h.
/// extregs hold VFP register bits, not host-native floating-point values.
#[repr(C)]
#[derive(Clone, Debug)]
pub struct AcxiAotContext {
    pub regs: [u32; 16],
    pub extregs: [u32; 64],
    pub cpsr: u32,
    pub fpscr: u32,
}

impl Default for AcxiAotContext {
    fn default() -> Self {
        Self { regs: [0; 16], extregs: [0; 64], cpsr: 0, fpscr: 0 }
    }
}

impl AcxiAotContext {
    pub fn new() -> Self { Self::default() }
}

const _: () = assert!(std::mem::size_of::<AcxiAotContext>() == 328);
const _: () = assert!(std::mem::offset_of!(AcxiAotContext, extregs) == 64);
const _: () = assert!(std::mem::offset_of!(AcxiAotContext, cpsr) == 320);
const _: () = assert!(std::mem::offset_of!(AcxiAotContext, fpscr) == 324);

extern "C" {
    pub fn acxi_aot_clear_exclusive();
    /// Runs only statically compiled application/library code and the exact
    /// dynamic-linker bridge patterns. Never interprets unknown game code.
    /// Returns -1 budget/step, -2 memory error, -3 unsupported instruction,
    /// -4 breakpoint, -5 untranslated address, or a nonnegative SVC immediate.
    pub fn acxi_aot_run(
        context: *mut AcxiAotContext,
        guest_memory: *mut u8,
        null_size: u32,
        ticks: *mut u64,
        single_step: bool,
    ) -> i32;
}
