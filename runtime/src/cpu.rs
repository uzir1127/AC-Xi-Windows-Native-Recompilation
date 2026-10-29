/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Native execution of statically translated ACXi ARM code.
//!
//! ACXi local fork, 2026-10-27: replaces Dynarmic with an ahead-of-time
//! generated C dispatcher. Unknown code addresses stop explicitly; there is
//! no interpreter or JIT fallback in this backend.

use crate::abi::GuestFunction;
use crate::mem::{GuestUSize, Mem};
use touchHLE_aot_wrapper::{acxi_aot_clear_exclusive, acxi_aot_run, AcxiAotContext};

type VAddr = u32;
pub type CpuContext = AcxiAotContext;

pub struct Cpu {
    context: CpuContext,
    /// When provided during construction, enforce the original binding to Mem.
    direct_memory_access_ptr: *mut std::ffi::c_void,
}

/// Why CPU execution ended.
#[derive(Debug)]
pub enum CpuState {
    /// Execution halted due to using up all remaining ticks (normal execution)
    /// or after the single instruction was executed (step execution).
    Normal,
    /// SVC instruction encountered.
    Svc(u32),
    /// An error was encountered.
    Error(CpuError),
}

/// A reason that can cause CPU execution to be interrupted.
#[derive(Debug, Clone, PartialEq)]
pub enum CpuError {
    /// Memory error during execution (probably a null page access).
    MemoryError,
    /// Undefined instruction (perhaps from a GDB software breakpoint).
    UndefinedInstruction,
    /// Breakpoint (`bkpt` instruction).
    Breakpoint,
}

impl Cpu {
    /// The register number of the stack pointer.
    pub const SP: usize = 13;
    /// The register number of the link register.
    #[allow(unused)]
    pub const LR: usize = 14;
    /// The register number of the program counter.
    pub const PC: usize = 15;

    /// When this bit is set in CPSR, the CPU is in Thumb mode.
    pub const CPSR_THUMB: u32 = 0x00000020;

    /// When this bit is set in CPSR, the CPU is in user mode.
    pub const CPSR_USER_MODE: u32 = 0x00000010;

    /// Construct a new CPU instance. If a mutable reference to a [Mem] instance
    /// is provided, direct memory access is enabled, and the CPU instance
    /// becomes bound to that [Mem] instance (subsequent calls must use the same
    /// one).
    pub fn new(direct_memory_access: Option<&mut Mem>) -> Cpu {
        unsafe { acxi_aot_clear_exclusive(); }
        let direct_memory_access_ptr = direct_memory_access
            .map_or(std::ptr::null_mut(), |mem| unsafe {
                mem.direct_memory_access_ptr()
            });
        Cpu { context: CpuContext::new(), direct_memory_access_ptr }
    }

    pub fn regs(&self) -> &[u32; 16] {
        &self.context.regs
    }
    pub fn regs_mut(&mut self) -> &mut [u32; 16] {
        &mut self.context.regs
    }

    /// Dump the registers of the current cpu to the log output.
    /// Silently ignores panics.
    pub fn dump_regs(&self) {
        let regs = self.regs();
        Self::echo_regs(regs);
    }

    pub fn echo_regs(regs: &[u32; 16]) {
        // Silently ignore panics so it's safe to use in contexts where we
        // can't panic.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for row in 0..4 {
                use std::fmt::Write;
                let mut line = String::new();
                for col in 0..4 {
                    let reg_idx = row * 4 + col;
                    match reg_idx {
                        Self::SP => write!(&mut line, "\t SP: "),
                        Self::LR => write!(&mut line, "\t LR: "),
                        Self::PC => write!(&mut line, "\t PC: "),
                        _ if reg_idx <= 9 => write!(&mut line, "\t R{reg_idx}: "),
                        _ => write!(&mut line, "\tR{reg_idx}: "),
                    }
                    .unwrap();
                    write!(&mut line, "{:#010x}", regs[reg_idx]).unwrap();
                }
                echo!("{}", line);
            }
        }));
    }

    pub fn cpsr(&self) -> u32 {
        self.context.cpsr
    }
    pub fn set_cpsr(&mut self, cpsr: u32) {
        self.context.cpsr = cpsr;
    }

    /// Swap the current state of the CPU (registers etc) with the state stored
    /// in the context object.
    pub fn swap_context(&mut self, context: &mut CpuContext) {
        unsafe { acxi_aot_clear_exclusive(); }
        std::mem::swap(&mut self.context, context);
    }

    /// Get PC with the Thumb bit appropriately set.
    pub fn pc_with_thumb_bit(&self) -> GuestFunction {
        let pc = self.regs()[Self::PC];
        let thumb = (self.cpsr() & Self::CPSR_THUMB) == Self::CPSR_THUMB;
        GuestFunction::from_addr_and_thumb_flag(pc, thumb)
    }

    /// Set PC and the Thumb flag for executing a guest function. Note that this
    /// does not touch LR.
    pub fn branch(&mut self, new_pc: GuestFunction) {
        self.regs_mut()[Self::PC] = new_pc.addr_without_thumb_bit();
        let cpsr_without_thumb = self.cpsr() & (!Self::CPSR_THUMB);
        self.set_cpsr(cpsr_without_thumb | ((new_pc.is_thumb() as u32) * Self::CPSR_THUMB))
    }

    /// Set the PC and Thumb flag (like [Self::branch]), but also set the LR,
    /// and return the original PC and LR.
    pub fn branch_with_link(
        &mut self,
        new_pc: GuestFunction,
        new_lr: GuestFunction,
    ) -> (GuestFunction, GuestFunction) {
        let old_pc = self.pc_with_thumb_bit();
        let old_lr = GuestFunction::from_addr_with_thumb_bit(self.regs()[Self::LR]);
        self.branch(new_pc);
        self.regs_mut()[Self::LR] = new_lr.addr_with_thumb_bit();
        (old_pc, old_lr)
    }

    /// The AOT backend has no instruction cache. The dynamic linker's
    /// import trampolines are recognized from their current guest-memory
    /// contents by the generated bridge dispatcher.
    pub fn invalidate_cache_range(&mut self, _base: VAddr, _size: GuestUSize) {}

    /// Start CPU execution.
    ///
    /// If `ticks` is [Some], it is used as an abstract time limit. The value
    /// will be reduced proportionately with the amount of ticks expended.
    ///
    /// If `ticks` is [None], the CPU executes only a single instruction. This
    /// is also known as "stepping".
    ///
    /// This will return either because the CPU ran out of time, or because
    /// something else happened which requires attention from the host.
    #[must_use]
    pub fn run_or_step(&mut self, mem: &mut Mem, ticks: Option<&mut u64>) -> CpuState {
        // See ::new() for why this is done.
        if !self.direct_memory_access_ptr.is_null() {
            assert!(self.direct_memory_access_ptr == unsafe { mem.direct_memory_access_ptr() });
        }

        let guest_memory = unsafe { mem.direct_memory_access_ptr() }.cast::<u8>();
        let single_step = ticks.is_none();
        let ticks_ptr = ticks.map_or(std::ptr::null_mut(), |value| value as *mut u64);
        #[cfg(feature = "profile-angle")]
        let native_scope = touchHLE_gl_bindings::profile::Scope::enter(400, "native_AOT");
        let res = unsafe {
            acxi_aot_run(
                &mut self.context,
                guest_memory,
                mem.null_segment_size(),
                ticks_ptr,
                single_step,
            )
        };
        #[cfg(feature = "profile-angle")]
        drop(native_scope);
        match res {
            -1 => CpuState::Normal,
            -2 => CpuState::Error(CpuError::MemoryError),
            -3 => CpuState::Error(CpuError::UndefinedInstruction),
            -4 => CpuState::Error(CpuError::Breakpoint),
            -5 => panic!(
                "ACXi AOT reached untranslated code at {:#010x} (Thumb={}); no interpreter fallback is enabled",
                self.context.regs[Self::PC],
                (self.context.cpsr & Self::CPSR_THUMB) != 0,
            ),
            _ if res < -5 => panic!("Unexpected AOT execution result {res}"),
            svc => CpuState::Svc(svc as u32),
        }
    }
}
