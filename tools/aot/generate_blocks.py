# ACXi port tools, 2026-10-29. SPDX-License-Identifier: MPL-2.0
"""Generate within-page static native control flow from AOT C.

This transformation preserves each semantic case verbatim. It replaces global
per-instruction dispatch with native labels, direct sequential gotos, and one
page-local dispatch at control-flow transfers. Budgets remain instruction based.
The input directory is read only and must differ from the output directory.
"""
from pathlib import Path
import argparse,re,sys,shutil,json,os

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('source',type=Path)
parser.add_argument('target',type=Path)
args=parser.parse_args()
source=args.source.resolve()
target=args.target.resolve()
if source==target:raise ValueError('Input and output must differ')
target.mkdir(exist_ok=True)
header=(source/'acxi_aot.h').read_text()
exithelper='\nstatic inline int32_t aot_page_exit(uint64_t*budget,uint64_t remaining,int32_t result){*budget=remaining;return result;}\n'
guard_end=header.rfind('#endif')
if guard_end<0:raise ValueError('Expected guarded generated header')
(target/'acxi_aot.h').write_text(header[:guard_end]+exithelper+header[guard_end:])
sources=(source/'sources.txt').read_text().splitlines()
for file in source.iterdir():
    if file.is_file() and file.name!='acxi_aot.h' and (file.suffix=='.h' or file.name in ('defines.txt','platform.h')):
        shutil.copyfile(file,target/file.name)
if (source/'include_dirs.txt').exists():
    dirs=[]
    for entry in (source/'include_dirs.txt').read_text().splitlines():
        if not entry.strip():continue
        dirs.append('.' if entry.strip()=='.' else os.path.relpath((source/entry).resolve(),target))
    (target/'include_dirs.txt').write_text('\n'.join(dirs)+'\n')
output_sources=[]
stats={'pages':0,'instruction_labels':0,'direct_sequential_edges':0,'control_flow_cases':0}
for name in sources:
    parts=Path(name).parts
    external=Path(name).is_absolute() or bool(parts and parts[0]=='..')
    output_sources.append(os.path.relpath((source/name).resolve(),target) if external else name)
    if not name.startswith('page_'):
        if name!='runtime.c' and not external:
            (target/name).parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(source/name,target/name)
        continue
    original=(source/name).read_text()
    page=int(name.split('_')[1].split('.')[0],16)
    namefn=name[:-2]
    starts=list(re.finditer(r'^case 0x([0-9a-f]+)u: /\* (.*?) \*/\n',original,re.M))
    cases={}
    for j,start in enumerate(starts):
        end=starts[j+1].start() if j+1<len(starts) else original.index('\ndefault:return -5;',start.end())
        body=original[start.end():end]
        assert body.rstrip().endswith('return err?-2:AOT_CONTINUE;')
        body=body[:body.rfind('return err?-2:AOT_CONTINUE;')].rstrip()
        key=int(start[1],16)
        nxt=int(re.search(r'c->regs\[15\]=0x([0-9a-f]+)u;',body)[1],16)|(key&1)
        cases[key]=(start[2],body,nxt)
    lines=['#include "acxi_aot.h"',f'int32_t {namefn}(AcxiContext*c,uint8_t*m,uint32_t null_size,uint64_t*budget){{',
           'uint32_t a,b,v,shc,addr,base;uint64_t wide;AotShift sh;int err=0;uint64_t remaining=*budget;',
           'dispatch:',f'if(!remaining)return aot_page_exit(budget,remaining,-1);',
           f'if((c->regs[15]>>12)!=0x{page:x}u)return aot_page_exit(budget,remaining,AOT_CONTINUE);',
           'switch(c->regs[15]|((c->cpsr>>5)&1u)){']
    for key in cases:lines.append(f'case 0x{key:x}u:goto L_{key:x};')
    lines+=['default:return aot_page_exit(budget,remaining,-5);','}']
    for key,(mnemonic,body,nxt) in cases.items():
        # Every early exit saves the precise remaining instruction budget.
        body=re.sub(r'\breturn ([^;]+);',r'return aot_page_exit(budget,remaining,\1);',body)
        lines += [f'L_{key:x}: /* {mnemonic} */','if(!remaining)return aot_page_exit(budget,remaining,-1);','--remaining;',body,
                  'if(err)return aot_page_exit(budget,remaining,-2);']
        control='aot_branch(' in body or body.count('c->regs[15]')>1
        if nxt in cases:
            if control:
                lines.append(f'if((c->regs[15]|((c->cpsr>>5)&1u))==0x{nxt:x}u)goto L_{nxt:x};')
                lines.append('goto dispatch;')
            else:
                lines.append(f'goto L_{nxt:x};')
            stats['direct_sequential_edges']+=1
        else:lines.append('goto dispatch;')
        stats['control_flow_cases']+=int(control)
    lines.append('}')
    (target/name).write_text('\n'.join(lines)+'\n')
    stats['pages']+=1;stats['instruction_labels']+=len(cases)

runtime=(source/'runtime.c').read_text()
runtime=re.sub(r'(int32_t page_[0-9a-f]+\(AcxiContext\*,uint8_t\*,uint32_t)\);',r'\1,uint64_t*);',runtime)
runtime=re.sub(r'(result=page_[0-9a-f]+\(c,m,null_size)\)',r'\1,&remaining)',runtime)
old_dispatch='if(result==-5)result=acxi_aot_trampoline(c,m,null_size);\n        --remaining;'
if runtime.count(old_dispatch)!=1:raise ValueError('Baseline dispatch changed; review budget transformation before generating')
runtime=runtime.replace(old_dispatch,
                        'if(result==-5){result=acxi_aot_trampoline(c,m,null_size);--remaining;}')
(target/'runtime.c').write_text(runtime)
(target/'sources.txt').write_text('\n'.join(output_sources)+'\n')
(target/'optimization.json').write_text(json.dumps(stats,indent=2)+'\n')
print(json.dumps(stats,indent=2))
