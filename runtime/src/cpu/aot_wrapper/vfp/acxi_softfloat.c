/* ACXi VFPv2 semantics, 2026-10-27. SPDX-License-Identifier: MPL-2.0
 * Arithmetic is implemented by Berkeley SoftFloat's ARM-VFPv2 specialization.
 * Its original BSD license and source notices must accompany this dependency.
 */
#include "platform.h"
#include "softfloat.h"
#include "acxi_softfloat.h"
#include <stdbool.h>

static bool nan32(uint32_t x) { return (x & 0x7fffffffu) > 0x7f800000u; }
static bool nan64(uint64_t x) { return (x & UINT64_C(0x7fffffffffffffff)) > UINT64_C(0x7ff0000000000000); }
static bool snan32(uint32_t x) { return nan32(x) && !(x & 0x00400000u); }
static bool snan64(uint64_t x) { return nan64(x) && !(x & UINT64_C(0x0008000000000000)); }
static unsigned mode(uint32_t fpscr) {
    static const unsigned modes[4] = {softfloat_round_near_even, softfloat_round_max, softfloat_round_min, softfloat_round_minMag};
    return modes[(fpscr >> 22) & 3];
}
static void begin(uint32_t fpscr) {
    softfloat_roundingMode = mode(fpscr);
    softfloat_detectTininess = softfloat_tininess_beforeRounding;
    softfloat_exceptionFlags = 0;
}
static void finish(uint32_t *fpscr) {
    unsigned f = softfloat_exceptionFlags;
    /* VFP flush-to-zero reports UFC without newly raising IXC for a tiny
     * result. Existing cumulative IXC remains set in the caller's FPSCR. */
    if ((*fpscr & (1u << 24)) && (f & softfloat_flag_underflow))
        f &= ~softfloat_flag_inexact;
    *fpscr |= ((f & softfloat_flag_invalid) ? 1u : 0u)
            | ((f & softfloat_flag_infinite) ? 2u : 0u)
            | ((f & softfloat_flag_overflow) ? 4u : 0u)
            | ((f & softfloat_flag_underflow) ? 8u : 0u)
            | ((f & softfloat_flag_inexact) ? 16u : 0u);
}
static uint32_t input32(uint32_t *fpscr, uint32_t x) {
    if ((*fpscr & (1u << 24)) && (x & 0x7fffffffu) && !(x & 0x7f800000u)) {
        *fpscr |= 0x80u;
        return x & 0x80000000u;
    }
    return x;
}
static uint64_t input64(uint32_t *fpscr, uint64_t x) {
    if ((*fpscr & (1u << 24)) && (x & UINT64_C(0x7fffffffffffffff)) && !(x & UINT64_C(0x7ff0000000000000))) {
        *fpscr |= 0x80u;
        return x & UINT64_C(0x8000000000000000);
    }
    return x;
}
static uint32_t output32(uint32_t *fpscr, uint32_t x) {
    if ((*fpscr & (1u << 25)) && nan32(x)) return 0x7fc00000u;
    if ((*fpscr & (1u << 24)) && (x & 0x7fffffffu) && !(x & 0x7f800000u)) {
        *fpscr |= 8u;
        return x & 0x80000000u;
    }
    return x;
}
static uint64_t output64(uint32_t *fpscr, uint64_t x) {
    if ((*fpscr & (1u << 25)) && nan64(x)) return UINT64_C(0x7ff8000000000000);
    if ((*fpscr & (1u << 24)) && (x & UINT64_C(0x7fffffffffffffff)) && !(x & UINT64_C(0x7ff0000000000000))) {
        *fpscr |= 8u;
        return x & UINT64_C(0x8000000000000000);
    }
    return x;
}
uint32_t acxi_vfp32(uint32_t *fpscr, int op, uint32_t x, uint32_t y) {
    float32_t a = {input32(fpscr, x)}, b = {input32(fpscr, y)}, r;
    begin(*fpscr);
    switch (op) {
    case 0: r = f32_add(a, b); break;
    case 1: r = f32_sub(a, b); break;
    case 2: r = f32_mul(a, b); break;
    case 3: r = f32_div(a, b); break;
    default: r = f32_sqrt(a); break;
    }
    finish(fpscr);
    return output32(fpscr, r.v);
}
uint64_t acxi_vfp64(uint32_t *fpscr, int op, uint64_t x, uint64_t y) {
    float64_t a = {input64(fpscr, x)}, b = {input64(fpscr, y)}, r;
    begin(*fpscr);
    switch (op) {
    case 0: r = f64_add(a, b); break;
    case 1: r = f64_sub(a, b); break;
    case 2: r = f64_mul(a, b); break;
    case 3: r = f64_div(a, b); break;
    default: r = f64_sqrt(a); break;
    }
    finish(fpscr);
    return output64(fpscr, r.v);
}
void acxi_vfp_compare(uint32_t *fpscr, uint64_t x, uint64_t y, int is64, int signal_nan) {
    uint32_t flags;
    begin(*fpscr);
    if (is64) {
        x = input64(fpscr, x); y = input64(fpscr, y);
        if (nan64(x) || nan64(y)) {
            if (signal_nan || snan64(x) || snan64(y)) softfloat_exceptionFlags |= softfloat_flag_invalid;
            flags = 0x30000000u;
        } else {
            float64_t a = {x}, b = {y};
            flags = f64_eq(a,b) ? 0x60000000u : f64_lt(a,b) ? 0x80000000u : 0x20000000u;
        }
    } else {
        uint32_t a_bits = input32(fpscr, (uint32_t)x), b_bits = input32(fpscr, (uint32_t)y);
        if (nan32(a_bits) || nan32(b_bits)) {
            if (signal_nan || snan32(a_bits) || snan32(b_bits)) softfloat_exceptionFlags |= softfloat_flag_invalid;
            flags = 0x30000000u;
        } else {
            float32_t a = {a_bits}, b = {b_bits};
            flags = f32_eq(a,b) ? 0x60000000u : f32_lt(a,b) ? 0x80000000u : 0x20000000u;
        }
    }
    *fpscr = (*fpscr & 0x0fffffffu) | flags;
    finish(fpscr);
}
uint32_t acxi_vfp_to_int(uint32_t *fpscr, uint64_t x, int is64, int uns, int truncate) {
    begin(*fpscr);
    unsigned rounding = truncate ? softfloat_round_minMag : mode(*fpscr);
    uint32_t r;
    if (is64) {
        float64_t a = {input64(fpscr, x)};
        r = uns ? (uint32_t)f64_to_ui32(a,rounding,true) : (uint32_t)f64_to_i32(a,rounding,true);
    } else {
        float32_t a = {input32(fpscr, (uint32_t)x)};
        r = uns ? (uint32_t)f32_to_ui32(a,rounding,true) : (uint32_t)f32_to_i32(a,rounding,true);
    }
    finish(fpscr);
    return r;
}
uint64_t acxi_int_to_vfp(uint32_t *fpscr, uint32_t x, int uns, int is64) {
    begin(*fpscr);
    uint64_t r;
    if (is64) { float64_t a = uns ? ui32_to_f64(x) : i32_to_f64((int32_t)x); r = a.v; }
    else { float32_t a = uns ? ui32_to_f32(x) : i32_to_f32((int32_t)x); r = a.v; }
    finish(fpscr);
    return r;
}
uint64_t acxi_vfp_convert(uint32_t *fpscr, uint64_t x, int source64) {
    begin(*fpscr);
    uint64_t r;
    if (source64) { float64_t a = {input64(fpscr,x)}; r = f64_to_f32(a).v; }
    else { float32_t a = {input32(fpscr,(uint32_t)x)}; r = f32_to_f64(a).v; }
    finish(fpscr);
    return source64 ? output32(fpscr,(uint32_t)r) : output64(fpscr,r);
}
