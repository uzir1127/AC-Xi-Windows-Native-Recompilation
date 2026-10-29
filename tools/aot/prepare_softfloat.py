# ACXi port tools, 2026-10-29. SPDX-License-Identifier: MPL-2.0
"""Attach the validated native ARM-VFPv2 arithmetic dependency to AOT output."""
from pathlib import Path
import os
import shutil


def prepare(output, runtime):
    runtime = Path(runtime).resolve()
    vendor = runtime / 'vendor' / 'softfloat'
    wrapper = runtime / 'src' / 'cpu' / 'aot_wrapper' / 'vfp'
    for name in ['acxi_softfloat.c', 'acxi_softfloat.h', 'platform.h']:
        shutil.copyfile(wrapper / name, output / name)
    include_dirs = ['.', os.path.relpath(vendor / 'source' / 'ARM-VFPv2', output),
                    os.path.relpath(vendor / 'source' / 'include', output)]
    (output / 'include_dirs.txt').write_text('\n'.join(include_dirs) + '\n')
    (output / 'defines.txt').write_text('\n'.join([
        'SOFTFLOAT_FAST_INT64=1', 'INLINE_LEVEL=5', 'SOFTFLOAT_ROUND_ODD=1',
        'SOFTFLOAT_FAST_DIV32TO16=1', 'SOFTFLOAT_FAST_DIV64TO32=1',
        'THREAD_LOCAL=_Thread_local',
    ]) + '\n')
    sources = ['acxi_softfloat.c']
    for line in (vendor / 'required-sources.txt').read_text().splitlines():
        if line.strip():
            sources.append(os.path.relpath(vendor / line.strip(), output))
    return sources
