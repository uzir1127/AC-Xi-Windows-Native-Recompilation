/* ACXi AOT dynamic-linker bridges, 2026-10-27. SPDX-License-Identifier: MPL-2.0
 *
 * This executes only exact bridge instructions emitted/restored by touchHLE's
 * dyld implementation. It is not an ARM instruction interpreter. Application
 * and library instructions must be statically translated by the generator.
 */
#include "acxi_aot.h"
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

/* Generated from Mach-O S_SYMBOL_STUBS sections, including library slides. */
extern bool acxi_aot_is_stub(uint32_t pc);

static bool bridge_word(const uint8_t *memory, uint32_t null_size,
                        uint64_t address, uint32_t *value) {
    if (address < null_size || address + 4u > UINT64_C(0x100000000))
        return false;
    memcpy(value, memory + address, 4);
    return true;
}

static void bridge_branch(AcxiContext *ctx, uint32_t target) {
    ctx->cpsr = (ctx->cpsr & ~UINT32_C(0x20)) | ((target & 1u) ? 0x20u : 0u);
    ctx->regs[15] = target & ((target & 1u) ? ~UINT32_C(1) : ~UINT32_C(3));
}

/* Return -6 after one ordinary bridge instruction, >=0 for SVC, -2 for
 * invalid memory, -3 for an intentional bridge trap, -5 for no known bridge.
 */
int32_t acxi_aot_trampoline(AcxiContext *ctx, uint8_t *memory, uint32_t null_size) {
    uint32_t pc = ctx->regs[15], word, next = 0, next2 = 0, previous = 0;
    if ((ctx->cpsr & 0x20u) || (pc & 3u))
        return -5;
    if (!bridge_word(memory, null_size, pc, &word))
        return -2;
    bool has_next = bridge_word(memory, null_size, (uint64_t)pc + 4u, &next);
    bool has_next2 = bridge_word(memory, null_size, (uint64_t)pc + 8u, &next2);
    bool has_previous = pc >= 4u && bridge_word(memory, null_size, pc - 4u, &previous);
    bool stub = acxi_aot_is_stub(pc);

    if ((word & 0xff000000u) == 0xef000000u &&
        (stub || (has_next && (next == 0xe12fff1eu || next == 0xe7ffdefeu)))) {
        ctx->regs[15] = pc + 4u;
        return (int32_t)(word & 0xffffffu);
    }

    if (word == 0xe12fff1eu && has_previous &&
        (previous & 0xff000000u) == 0xef000000u) {
        bridge_branch(ctx, ctx->regs[14]);
        return -6;
    }

    if (word == 0xe7ffdefeu && has_previous &&
        (previous & 0xff000000u) == 0xef000000u) {
        ctx->regs[15] = pc + 4u;
        return -3;
    }

    /* Restored original stubs may jump to statically translated library code.
     * Restrict these patterns to the generated symbol-stub ranges.
     */
    if (!stub)
        return -5;

    if (word == 0xe59fc000u && has_next && next == 0xe59cf000u) {
        if (!bridge_word(memory, null_size, (uint64_t)pc + 8u, &ctx->regs[12]))
            return -2;
        ctx->regs[15] = pc + 4u;
        return -6;
    }

    if (word == 0xe59fc004u && has_next && has_next2 &&
        next == 0xe08fc00cu && next2 == 0xe59cf000u) {
        if (!bridge_word(memory, null_size, (uint64_t)pc + 12u, &ctx->regs[12]))
            return -2;
        ctx->regs[15] = pc + 4u;
        return -6;
    }

    if (word == 0xe08fc00cu && has_previous && has_next &&
        previous == 0xe59fc004u && next == 0xe59cf000u) {
        ctx->regs[12] += pc + 8u;
        ctx->regs[15] = pc + 4u;
        return -6;
    }

    if (word == 0xe59cf000u && has_previous &&
        (previous == 0xe59fc000u || previous == 0xe08fc00cu)) {
        uint32_t target;
        if (!bridge_word(memory, null_size, ctx->regs[12], &target))
            return -2;
        bridge_branch(ctx, target);
        return -6;
    }

    if ((word & 0xfffff000u) == 0xe59ff000u) {
        uint32_t target;
        if (!bridge_word(memory, null_size,
                         (uint64_t)pc + 8u + (word & 0xfffu), &target))
            return -2;
        bridge_branch(ctx, target);
        return -6;
    }

    return -5;
}
