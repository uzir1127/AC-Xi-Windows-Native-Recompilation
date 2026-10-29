/* ACXi local port: adapted from the user's ACX hardware counter implementation.
 * SPDX-License-Identifier: MPL-2.0 */
/* Windows counters are queried on a sleeping worker, never on the renderer.
 * PDH is loaded from Windows itself; no vendor tools or extra DLLs needed. */
#include "acxi_hardware.h"
#include <SDL.h>
#include <atomic>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <cwchar>
#include <map>
#include <vector>
#include <mutex>
#include <cstdlib>
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan_core.h>
#if defined(__x86_64__) || defined(__i386__)
#include <cpuid.h>
#endif
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <psapi.h>
#include <pdh.h>
#include <pdhmsg.h>
#endif

static HardwareStats s_stats;
static SDL_mutex *s_mutex;
static SDL_sem *s_wake;
static SDL_Thread *s_thread;
static std::atomic<bool> s_running{false}, s_visible{false};
static uint32_t s_luid_high, s_luid_low;
static bool s_luid_valid;
static char s_nvidia_pci[32];

bool hardware_gpu_usage(const HardwareGpuSample *samples, unsigned count,
                        uint32_t high, uint32_t low, double *out)
{
    std::map<uint64_t, double> engines;
    for (unsigned i = 0; i < count; ++i) {
        const auto &s = samples[i];
        if (!s.valid || !s.instance || !std::isfinite(s.percent) || s.percent < 0) continue;
        const wchar_t *luid = wcsstr(s.instance, L"luid_0x");
        unsigned h, l, physical, engine;
        if (!luid || swscanf(luid, L"luid_0x%x_0x%x_phys_%u_eng_%u", &h, &l, &physical, &engine) != 4)
            continue;
        if (h != high || l != low) continue;
        engines[(uint64_t(physical) << 32) | engine] += s.percent;
    }
    if (engines.empty()) return false;
    double busy = 0;
    for (const auto &e : engines) if (e.second > busy) busy = e.second;
    *out = busy > 100 ? 100 : busy;
    return true;
}

static void cpu_name(char *name, size_t size)
{
    char brand[49] = {};
#if defined(__x86_64__) || defined(__i386__)
    unsigned a, b, c, d;
    if (__get_cpuid(0x80000000u, &a, &b, &c, &d) && a >= 0x80000004u) {
        for (unsigned i = 0; i < 3; ++i) {
            __get_cpuid(0x80000002u + i, &a, &b, &c, &d);
            unsigned words[] = {a,b,c,d};
            memcpy(brand + i * 16, words, 16);
        }
    }
#endif
    /* Collapse the padding commonly present in the CPU brand string. */
    size_t n = 0;
    for (const char *p = brand; *p && n + 1 < size; ++p) {
        if (*p == ' ' && (!n || name[n-1] == ' ')) continue;
        name[n++] = *p;
    }
    while (n && name[n-1] == ' ') --n;
    name[n] = 0;
    if (!n) snprintf(name, size, "Processor (%d logical cores)", SDL_GetCPUCount());
}

#ifdef _WIN32
struct Counters {
    HMODULE dll = nullptr;
    PDH_HQUERY query = nullptr;
    PDH_HCOUNTER cpu = nullptr, gpu = nullptr;
    decltype(&PdhOpenQueryW) open = nullptr;
    decltype(&PdhAddEnglishCounterW) add = nullptr;
    decltype(&PdhCollectQueryData) collect = nullptr;
    decltype(&PdhGetFormattedCounterValue) value = nullptr;
    decltype(&PdhGetFormattedCounterArrayW) array = nullptr;
    decltype(&PdhCloseQuery) close = nullptr;

    void init() {
        dll = LoadLibraryExW(L"pdh.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if (!dll) return;
#define PDH_FN(member, name) member = reinterpret_cast<decltype(member)>(GetProcAddress(dll, #name))
        PDH_FN(open, PdhOpenQueryW); PDH_FN(add, PdhAddEnglishCounterW);
        PDH_FN(collect, PdhCollectQueryData); PDH_FN(value, PdhGetFormattedCounterValue);
        PDH_FN(array, PdhGetFormattedCounterArrayW); PDH_FN(close, PdhCloseQuery);
#undef PDH_FN
        if (!open || !add || !collect || !value || !array || !close) return;
        PDH_STATUS status = open(nullptr, 0, &query);
        fprintf(stderr, "[hardware] PDH query: %08lx\n", (unsigned long)status);
        if (status != ERROR_SUCCESS) { query = nullptr; return; }
        status = add(query, L"\\Processor Information(_Total)\\% Processor Time", 0, &cpu);
        fprintf(stderr, "[hardware] CPU counter: %08lx\n", (unsigned long)status);
        if (status != ERROR_SUCCESS) cpu = nullptr;
        if (s_luid_valid) {
            status = add(query, L"\\GPU Engine(*)\\Utilization Percentage", 0, &gpu);
            fprintf(stderr, "[hardware] GPU counter: %08lx\n", (unsigned long)status);
            if (status != ERROR_SUCCESS) gpu = nullptr;
        }
    }
    ~Counters() { if (query && close) close(query); if (dll) FreeLibrary(dll); }
    bool cpu_usage(double *out) {
        PDH_FMT_COUNTERVALUE v = {};
        if (!cpu || value(cpu, PDH_FMT_DOUBLE, nullptr, &v) != ERROR_SUCCESS ||
            (v.CStatus != PDH_CSTATUS_VALID_DATA && v.CStatus != PDH_CSTATUS_NEW_DATA) ||
            !std::isfinite(v.doubleValue) || v.doubleValue < 0) return false;
        *out = v.doubleValue > 100 ? 100 : v.doubleValue;
        return true;
    }
    bool gpu_usage(double *out) {
        if (!gpu) return false;
        /* Instances can appear/disappear during collection. Start with a
         * fresh size query on each retry, as required by PDH. */
        for (int retry = 0; retry < 3; ++retry) {
            DWORD bytes = 0, count = 0;
            if (array(gpu, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &bytes, &count, nullptr) != static_cast<PDH_STATUS>(PDH_MORE_DATA) ||
                !bytes || bytes > 32 * 1024 * 1024) return false;
            std::vector<unsigned char> buffer(bytes);
            auto items = reinterpret_cast<PDH_FMT_COUNTERVALUE_ITEM_W *>(buffer.data());
            PDH_STATUS status = array(gpu, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &bytes, &count, items);
            if (status == static_cast<PDH_STATUS>(PDH_MORE_DATA)) continue;
            if (status != ERROR_SUCCESS) return false;
            std::vector<HardwareGpuSample> samples;
            samples.reserve(count);
            for (DWORD i = 0; i < count; ++i) {
                const auto &v = items[i].FmtValue;
                samples.push_back({items[i].szName, v.doubleValue,
                    v.CStatus == PDH_CSTATUS_VALID_DATA || v.CStatus == PDH_CSTATUS_NEW_DATA});
            }
            return hardware_gpu_usage(samples.data(), unsigned(samples.size()), s_luid_high, s_luid_low, out);
        }
        return false;
    }
};

/* The CPU busy-time fallback needs no performance counter providers. On
 * multi-group machines retain the all-group PDH reading instead of showing
 * a misleading partial-system fallback. */
struct CpuBusy {
    uint64_t idle = 0, total = 0;
    bool primed = false;
    void reset() { primed = false; }
    bool sample(double *out) {
        FILETIME i, k, u;
        if (GetActiveProcessorGroupCount() > 1 || !GetSystemTimes(&i, &k, &u)) return false;
        auto ticks = [](FILETIME t) { return (uint64_t(t.dwHighDateTime) << 32) | t.dwLowDateTime; };
        uint64_t ni = ticks(i), nt = ticks(k) + ticks(u);
        bool valid = primed && nt > total && ni >= idle && ni - idle <= nt - total;
        if (valid) *out = 100.0 * (double)(nt - total - (ni - idle)) / (double)(nt - total);
        idle = ni; total = nt; primed = true;
        return valid;
    }
};

/* NVML is supplied by NVIDIA's installed driver. Match its PCI bus to the
 * Vulkan rendering adapter; never assume NVIDIA device index zero. */
struct NvidiaUsage {
    struct Util { unsigned gpu, memory; };
    HMODULE dll = nullptr;
    void *device = nullptr;
    bool initialized = false;
    int (*init)() = nullptr;
    int (*shutdown)() = nullptr;
    int (*by_bus)(const char *, void **) = nullptr;
    int (*utilization)(void *, Util *) = nullptr;
    void open() {
        if (!s_nvidia_pci[0]) return;
        dll = LoadLibraryExW(L"nvml.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if (!dll) return;
        init = reinterpret_cast<decltype(init)>(GetProcAddress(dll, "nvmlInit_v2"));
        shutdown = reinterpret_cast<decltype(shutdown)>(GetProcAddress(dll, "nvmlShutdown"));
        by_bus = reinterpret_cast<decltype(by_bus)>(GetProcAddress(dll, "nvmlDeviceGetHandleByPciBusId_v2"));
        utilization = reinterpret_cast<decltype(utilization)>(GetProcAddress(dll, "nvmlDeviceGetUtilizationRates"));
        if (!init || !shutdown || !by_bus || !utilization || init() != 0) return;
        initialized = true;
        if (by_bus(s_nvidia_pci, &device) != 0) device = nullptr;
        fprintf(stderr, "[hardware] NVIDIA driver telemetry for %s: %s\n", s_nvidia_pci, device ? "available" : "unavailable");
    }
    bool sample(double *out) {
        Util v = {};
        if (!device || utilization(device, &v) != 0 || v.gpu > 100) return false;
        *out = v.gpu;
        return true;
    }
    ~NvidiaUsage() { if (initialized) shutdown(); if (dll) FreeLibrary(dll); }
};
#endif

static int worker(void *)
{
    SDL_SetThreadPriority(SDL_THREAD_PRIORITY_LOW);
#ifdef _WIN32
    Counters counters;
    counters.init();
    CpuBusy cpu_busy;
    NvidiaUsage nvidia;
    nvidia.open();
    bool collection_logged = false;
#endif
    bool primed = false;
    while (s_running.load()) {
        if (!s_visible.load()) {
            primed = false;
#ifdef _WIN32
            cpu_busy.reset();
#endif
            SDL_SemWait(s_wake); continue;
        }
        HardwareStats next = {};
#ifdef _WIN32
        PDH_STATUS collection = counters.query ? counters.collect(counters.query) : static_cast<PDH_STATUS>(PDH_INVALID_HANDLE);
        bool collected = collection == ERROR_SUCCESS;
        if (!collection_logged) {
            fprintf(stderr, "[hardware] first collection: %08lx\n", (unsigned long)collection);
            collection_logged = true;
        }
        if (primed && collected) {
            next.cpu_valid = counters.cpu_usage(&next.cpu_percent);
            next.gpu_valid = counters.gpu_usage(&next.gpu_percent);
        }
        double cpu_fallback = 0;
        bool cpu_fallback_valid = cpu_busy.sample(&cpu_fallback);
        if (!next.cpu_valid && cpu_fallback_valid) { next.cpu_percent = cpu_fallback; next.cpu_valid = true; }
        double gpu_driver = 0;
        if (nvidia.sample(&gpu_driver)) { next.gpu_percent = gpu_driver; next.gpu_valid = true; }
        MEMORYSTATUSEX memory = {}; memory.dwLength = sizeof memory;
        if (GlobalMemoryStatusEx(&memory)) {
            next.ram_total = memory.ullTotalPhys;
            next.ram_used = memory.ullTotalPhys - memory.ullAvailPhys;
        }
        PROCESS_MEMORY_COUNTERS process = {}; process.cb = sizeof process;
        if (GetProcessMemoryInfo(GetCurrentProcess(), &process, sizeof process))
            next.process_bytes = process.WorkingSetSize;
        primed = collected;
#else
        (void)primed;
#endif
        SDL_LockMutex(s_mutex);
        s_stats.cpu_percent = next.cpu_percent; s_stats.cpu_valid = next.cpu_valid;
        s_stats.gpu_percent = next.gpu_percent; s_stats.gpu_valid = next.gpu_valid;
        s_stats.ram_total = next.ram_total; s_stats.ram_used = next.ram_used;
        s_stats.process_bytes = next.process_bytes;
        s_stats.sampled_at = SDL_GetTicks64();
        SDL_UnlockMutex(s_mutex);
        SDL_SemWaitTimeout(s_wake, 1000);
    }
    return 0;
}

void hardware_stats_init(const char *gpu_name, const uint8_t *luid, bool valid,
                         const char *nvidia_pci_bus)
{
    s_stats = {};
    cpu_name(s_stats.cpu_name, sizeof s_stats.cpu_name);
    snprintf(s_stats.gpu_name, sizeof s_stats.gpu_name, "%s", gpu_name && *gpu_name ? gpu_name : "Unknown GPU");
    s_luid_valid = valid && luid;
    snprintf(s_nvidia_pci, sizeof s_nvidia_pci, "%s", nvidia_pci_bus ? nvidia_pci_bus : "");
    if (s_luid_valid) { memcpy(&s_luid_low, luid, 4); memcpy(&s_luid_high, luid + 4, 4); }
    fprintf(stderr, "[hardware] CPU: %s; GPU: %s; GPU counter matching %s\n",
            s_stats.cpu_name, s_stats.gpu_name, s_luid_valid ? "enabled" : "unavailable");
    s_mutex = SDL_CreateMutex(); s_wake = SDL_CreateSemaphore(0);
    if (!s_mutex || !s_wake) return;
    s_visible.store(false); s_running.store(true);
    s_thread = SDL_CreateThread(worker, "Hardware usage", nullptr);
    if (!s_thread) s_running.store(false);
}

void hardware_stats_set_visible(bool visible)
{
    if (s_visible.exchange(visible) != visible && s_wake) SDL_SemPost(s_wake);
}

HardwareStats hardware_stats_read(void)
{
    if (s_mutex) SDL_LockMutex(s_mutex);
    HardwareStats copy = s_stats;
    if (s_mutex) SDL_UnlockMutex(s_mutex);
    if (SDL_GetTicks64() - copy.sampled_at > 3000) copy.cpu_valid = copy.gpu_valid = false;
    return copy;
}

void hardware_stats_shutdown(void)
{
    s_running.store(false);
    if (s_wake) SDL_SemPost(s_wake);
    if (s_thread) SDL_WaitThread(s_thread, nullptr);
    if (s_wake) SDL_DestroySemaphore(s_wake);
    if (s_mutex) SDL_DestroyMutex(s_mutex);
    s_thread = nullptr; s_wake = nullptr; s_mutex = nullptr;
}

// ANGLE supplies the physical device actually backing its Vulkan renderer.
// Only read-only identity/property queries run here; no extra device or queue
// is created and the worker never accesses the Vulkan renderer.
extern "C" void acxi_hardware_init(void *physical, void *instance, void *get_instance_proc)
{
    static std::once_flag once;
    std::call_once(once, [=] {
        VkPhysicalDeviceProperties properties = {};
        VkPhysicalDeviceIDProperties ids = {};
        ids.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES;
        VkPhysicalDevicePCIBusInfoPropertiesEXT pci = {};
        pci.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PCI_BUS_INFO_PROPERTIES_EXT;
        char bus[32] = {};
        auto get = reinterpret_cast<PFN_vkGetInstanceProcAddr>(get_instance_proc);
        if (physical && instance && get) {
            auto device = reinterpret_cast<VkPhysicalDevice>(physical);
            auto inst = reinterpret_cast<VkInstance>(instance);
            auto query = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties>(get(inst, "vkGetPhysicalDeviceProperties"));
            auto query2 = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties2>(get(inst, "vkGetPhysicalDeviceProperties2"));
            auto extensions = reinterpret_cast<PFN_vkEnumerateDeviceExtensionProperties>(get(inst, "vkEnumerateDeviceExtensionProperties"));
            if (query) query(device, &properties);
            bool pci_available = false;
            uint32_t count = 0;
            if (extensions && extensions(device, nullptr, &count, nullptr) == VK_SUCCESS && count <= 4096) {
                std::vector<VkExtensionProperties> names(count);
                if (extensions(device, nullptr, &count, names.data()) == VK_SUCCESS)
                    for (const auto &name : names)
                        if (!strcmp(name.extensionName, VK_EXT_PCI_BUS_INFO_EXTENSION_NAME)) pci_available = true;
            }
            if (query2) {
                VkPhysicalDeviceProperties2 info = {};
                info.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;
                info.pNext = &ids;
                if (pci_available) ids.pNext = &pci;
                query2(device, &info);
                if (pci_available && properties.vendorID == 0x10de)
                    snprintf(bus, sizeof bus, "%04x:%02x:%02x.%u", pci.pciDomain, pci.pciBus, pci.pciDevice, pci.pciFunction);
            }
        }
        hardware_stats_init(properties.deviceName, ids.deviceLUID, ids.deviceLUIDValid != 0, bus);
        if (s_mutex) SDL_LockMutex(s_mutex);
        s_stats.api_version = properties.apiVersion;
        if (s_mutex) SDL_UnlockMutex(s_mutex);
        std::atexit(hardware_stats_shutdown);
    });
}
extern "C" void acxi_hardware_visible(bool visible) { hardware_stats_set_visible(visible); }
extern "C" void acxi_hardware_read(HardwareStats *out) { if (out) *out = hardware_stats_read(); }
extern "C" void acxi_hardware_shutdown() { hardware_stats_shutdown(); }
