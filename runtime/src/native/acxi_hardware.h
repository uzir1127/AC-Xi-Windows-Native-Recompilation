/* SPDX-License-Identifier: MPL-2.0 */
#pragma once
#include <stdint.h>
#include <wchar.h>

struct HardwareStats {
    char cpu_name[128], gpu_name[256];
    double cpu_percent, gpu_percent;
    bool cpu_valid, gpu_valid;
    uint64_t sampled_at, ram_total, ram_used, process_bytes;
    uint32_t api_version;
};
static_assert(sizeof(HardwareStats) == 448, "Rust snapshot ABI changed");
struct HardwareGpuSample { const wchar_t *instance; double percent; bool valid; };
bool hardware_gpu_usage(const HardwareGpuSample *, unsigned, uint32_t, uint32_t, double *);
void hardware_stats_init(const char *, const uint8_t *, bool, const char *);
void hardware_stats_set_visible(bool);
HardwareStats hardware_stats_read();
void hardware_stats_shutdown();

extern "C" {
void acxi_hardware_init(void *physical, void *instance, void *get_instance_proc);
void acxi_hardware_visible(bool);
void acxi_hardware_read(HardwareStats *);
void acxi_hardware_shutdown();
}
