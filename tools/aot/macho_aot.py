# ACXi port tools, 2026-10-29. SPDX-License-Identifier: MPL-2.0
"""Mach-O ARM code inventory for the ACXi ahead-of-time compiler."""
import sys, struct, zipfile, plistlib, collections, json
from pathlib import Path
import capstone as cs
from capstone import arm_const as arm

class Image:
    def __init__(self, path, slide=0):
        self.path=Path(path); self.slide=slide; self.name=self.path.stem
        raw=self.path.read_bytes()
        if self.path.suffix.lower()=='.ipa':
            with zipfile.ZipFile(self.path) as z:
                pn=next(n for n in z.namelist() if n.startswith('Payload/') and n.count('/')==2 and n.endswith('.app/Info.plist'))
                pl=plistlib.loads(z.read(pn)); raw=z.read(pn.rsplit('/',1)[0]+'/'+pl['CFBundleExecutable'])
        if raw[:4]==b'\xca\xfe\xba\xbe':
            n=struct.unpack_from('>I',raw,4)[0]
            arches=[struct.unpack_from('>5I',raw,8+i*20) for i in range(n)]
            a=next((a for a in arches if a[0]==12 and a[1]==6),next(a for a in arches if a[0]==12))
            raw=raw[a[2]:a[2]+a[3]]
        if raw[:4]!=b'\xce\xfa\xed\xfe':raise ValueError('Expected 32-bit little-endian Mach-O')
        self.data=raw; self.sections=[]; self.segments=[]; self.symbols=[]; self.entry=None
        off=28; sym=None
        for i in range(struct.unpack_from('<I',raw,16)[0]):
            cmd,size=struct.unpack_from('<II',raw,off)
            if cmd==1:
                sg=struct.unpack_from('<II16s8I',raw,off)
                self.segments.append({'name':sg[2].split(b'\0')[0].decode(),'addr':sg[3],'size':sg[4],'offset':sg[5],'file_size':sg[6]})
                for j in range(sg[-2]):
                    s=struct.unpack_from('<16s16s9I',raw,off+56+j*68)
                    self.sections.append({'name':s[0].split(b'\0')[0].decode(),'segment':s[1].split(b'\0')[0].decode(),'addr':s[2],'size':s[3],'offset':s[4],'flags':s[8]})
            elif cmd==2:sym=struct.unpack_from('<4I',raw,off+8)
            elif cmd==5:self.entry=struct.unpack_from('<I',raw,off+76)[0]
            off+=size
        if sym:
            so,n,st,ss=sym
            for i in range(n):
                idx,t,se,d,a=struct.unpack_from('<IBBHI',raw,so+i*12)
                if t&0xe0 or idx>=ss:continue
                name=raw[st+idx:st+ss].split(b'\0')[0].decode('utf8','replace')
                self.symbols.append((name,t,se,d,a))
        self.code=[s for s in self.sections if s['flags']&0x80000000 and 'stub' not in s['name']]
        self.stub_sections=[s for s in self.sections if 'stub' in s['name']]
        self.text_symbols={}
        for name,t,se,d,a in self.symbols:
            if t&14==14 and se and self.sections[se-1] in self.code:
                self.text_symbols[a&~1]=(name,bool(d&8 or a&1))

    def locate(self, addr):
        for s in self.code:
            if s['addr']<=addr<s['addr']+s['size']:return s
    def read(self,addr,size):
        for s in self.sections:
            if s['offset'] and s['addr']<=addr and addr+size<=s['addr']+s['size']:
                o=s['offset']+addr-s['addr'];return self.data[o:o+size]
        return b''

    def discover(self):
        dis={False:cs.Cs(cs.CS_ARCH_ARM,cs.CS_MODE_ARM|cs.CS_MODE_LITTLE_ENDIAN),True:cs.Cs(cs.CS_ARCH_ARM,cs.CS_MODE_THUMB|cs.CS_MODE_LITTLE_ENDIAN)}
        for d in dis.values():d.detail=True
        seeds=[(a,t) for a,(_,t) in self.text_symbols.items()]
        if self.entry is not None:seeds.append((self.entry&~1,bool(self.entry&1)))
        # Constructors and Objective-C methods normally have symbols in this
        # build. Data pointers add conservative seeds for stripped guest libs.
        for sec in self.sections:
            if sec['name'] in ('__mod_init_func',):
                raw=self.data[sec['offset']:sec['offset']+sec['size']]
                for i in range(0,len(raw)-3,4):
                    a=struct.unpack_from('<I',raw,i)[0]
                    if self.locate(a&~1):seeds.append((a&~1,bool(a&1)))
        todo=list(seeds); found={};bad=[]
        while todo:
            pc,thumb=todo.pop()
            while (pc,thumb) not in found and self.locate(pc):
                raw=self.read(pc,4)
                ins=next(dis[thumb].disasm(raw,pc,count=1),None)
                if ins is None:bad.append((hex(pc),thumb,raw.hex()));break
                found[(pc,thumb)]=ins
                nxt=pc+ins.size; cond=ins.cc not in (arm.ARM_CC_AL,arm.ARM_CC_INVALID)
                m=ins.mnemonic.split('.')[0]
                if ins.group(cs.CS_GRP_CALL):
                    if ins.operands and ins.operands[0].type==arm.ARM_OP_IMM:
                        t=ins.operands[0].imm; tm=(not thumb) if m.startswith('blx') else thumb
                        if self.locate(t&~1):todo.append((t&~1,tm))
                elif ins.group(cs.CS_GRP_JUMP):
                    if ins.operands and ins.operands[0].type==arm.ARM_OP_IMM:
                        t=ins.operands[0].imm
                        if self.locate(t&~1):todo.append((t&~1,thumb))
                    if not cond:break
                else:
                    try:writes=ins.regs_access()[1]
                    except cs.CsError:writes=[]
                    if arm.ARM_REG_PC in writes and not cond:break
                    if m in ('udf','bkpt'):break
                pc=nxt
        self.instructions=found;self.bad=bad
        return found

    def report(self):
        if not hasattr(self,'instructions'):self.discover()
        hist=collections.Counter(i.mnemonic for i in self.instructions.values())
        allbytes=sum(s['size'] for s in self.code)
        covered=set()
        for (pc,t),i in self.instructions.items():covered.update(range(pc,pc+i.size))
        return {'path':str(self.path),'slide':hex(self.slide),'symbol_seeds':len(self.text_symbols),'instruction_count':len(self.instructions),'code_bytes':allbytes,'reachable_bytes':len(covered),'reachable_fraction':len(covered)/allbytes,'arm_instructions':sum(not t for a,t in self.instructions),'thumb_instructions':sum(t for a,t in self.instructions),'decode_failures':self.bad[:20],'mnemonics':dict(hist.most_common())}

if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ipa',type=Path)
    parser.add_argument('--runtime',type=Path,default=Path(__file__).resolve().parents[2]/'runtime')
    args=parser.parse_args()
    ipa=Image(args.ipa)
    runtime=args.runtime/'touchHLE_dylibs'
    imgs=[ipa,Image(runtime/'libgcc_s.1.dylib',0x30000000),Image(runtime/'libstdc++.6.0.9.dylib',0x3748a000)]
    out=[]
    for image in imgs:
        r=image.report();out.append(r);print(json.dumps(r,indent=2),flush=True)
