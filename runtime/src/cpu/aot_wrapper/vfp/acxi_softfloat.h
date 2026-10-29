/* ACXi VFPv2 semantics, 2026-10-27. SPDX-License-Identifier: MPL-2.0 */
#ifndef ACXI_SOFTFLOAT_H
#define ACXI_SOFTFLOAT_H
#include <stdint.h>
/* Arithmetic op: 0 add, 1 sub, 2 mul, 3 div, 4 sqrt. Arguments/results are bits. */
uint32_t acxi_vfp32(uint32_t *fpscr, int op, uint32_t x, uint32_t y);
uint64_t acxi_vfp64(uint32_t *fpscr, int op, uint64_t x, uint64_t y);
void acxi_vfp_compare(uint32_t *fpscr, uint64_t x, uint64_t y, int is64, int signal_nan);
uint32_t acxi_vfp_to_int(uint32_t *fpscr, uint64_t x, int is64, int uns, int truncate);
uint64_t acxi_int_to_vfp(uint32_t *fpscr, uint32_t x, int uns, int is64);
uint64_t acxi_vfp_convert(uint32_t *fpscr, uint64_t x, int source64);
#endif
