# ACXi port tools, 2026-10-29. SPDX-License-Identifier: MPL-2.0
"""Compile ACXi ARM machine instructions into portable, resumable native C.

All machine decoding happens here, before compilation. Unsupported operations
emit explicit traps; there is no general instruction interpreter in the output.
"""
from pathlib import Path
import argparse,collections,hashlib,json,struct,sys,shutil
from macho_aot import Image,cs,arm

ROOT=Path(__file__).resolve().parent
OUT=None
COND={1:'eq',2:'ne',3:'hs',4:'lo',5:'mi',6:'pl',7:'vs',8:'vc',9:'hi',10:'ls',11:'ge',12:'lt',13:'gt',14:'le'}
ALIASES={'cs':'hs','cc':'lo'}

HEADER=r'''#ifndef ACXI_AOT_H
#define ACXI_AOT_H
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <math.h>
#include <limits.h>
typedef struct AcxiContext { uint32_t regs[16]; uint32_t extregs[64]; uint32_t cpsr; uint32_t fpscr; } AcxiContext;
int32_t acxi_aot_run(AcxiContext*,uint8_t*,uint32_t,uint64_t*,bool);
int32_t acxi_aot_trampoline(AcxiContext*,uint8_t*,uint32_t);
bool acxi_aot_is_stub(uint32_t);
void acxi_aot_clear_exclusive(void);
void aot_exclusive_invalidate(uint32_t,unsigned);
uint32_t aot_ldrex(AcxiContext*,uint8_t*,uint32_t,uint32_t,int*);
uint32_t aot_strex(AcxiContext*,uint8_t*,uint32_t,uint32_t,uint32_t,int*);
#define AOT_CONTINUE (-100)
static inline int aot_cond(uint32_t f,int c){int n=f>>31,z=(f>>30)&1,cy=(f>>29)&1,v=(f>>28)&1;switch(c){case 0:return z;case 1:return !z;case 2:return cy;case 3:return !cy;case 4:return n;case 5:return !n;case 6:return v;case 7:return !v;case 8:return cy&&!z;case 9:return !cy||z;case 10:return n==v;case 11:return n!=v;case 12:return !z&&n==v;case 13:return z||n!=v;default:return 1;}}
static inline void aot_nz(AcxiContext*c,uint32_t v){c->cpsr=(c->cpsr&0x3fffffffu)|(v&0x80000000u)|(v?0:0x40000000u);}
static inline void aot_logic(AcxiContext*c,uint32_t v,uint32_t cy){c->cpsr=(c->cpsr&0x1fffffffu)|(v&0x80000000u)|(v?0:0x40000000u)|((cy&1u)<<29);}
static inline uint32_t aot_add(AcxiContext*c,uint32_t a,uint32_t b,uint32_t cy,int flags){uint64_t w=(uint64_t)a+b+cy;uint32_t v=(uint32_t)w;if(flags)c->cpsr=(c->cpsr&0x0fffffffu)|(v&0x80000000u)|(v?0:0x40000000u)|((uint32_t)(w>>32)<<29)|((~(a^b)&(a^v)&0x80000000u)>>3);return v;}
typedef struct {uint32_t v,c;} AotShift;
static inline AotShift aot_shift(uint32_t v,unsigned type,unsigned n,int reg,unsigned cy){AotShift s={v,cy};if(reg&&n==0)return s;switch(type){case 0:if(!n)return s;s.c=n<32?(v>>(32-n))&1:n==32?v&1:0;s.v=n<32?v<<n:0;break;case 1:if(!reg&&!n)n=32;s.c=n<32?(v>>(n-1))&1:n==32?v>>31:0;s.v=n<32?v>>n:0;break;case 2:if(!reg&&!n)n=32;s.c=n<32?(v>>(n-1))&1:v>>31;s.v=n<32?(uint32_t)((int32_t)v>>n):(v&0x80000000u?0xffffffffu:0);break;default:if(!reg&&!n){s.c=v&1;s.v=(cy<<31)|(v>>1);}else{n&=31;s.v=n?(v>>n)|(v<<(32-n)):v;s.c=s.v>>31;}break;}return s;}
static inline uint32_t aot_ror(uint32_t v,unsigned n){n&=31;return n?(v>>n)|(v<<(32-n)):v;}
static inline uint64_t aot_load(uint8_t*m,uint32_t a,unsigned n,uint32_t null_size,int*err){uint64_t v=0;if(a<null_size||(uint64_t)a+n>0x100000000ull){*err=1;return 0;}memcpy(&v,m+(uint64_t)a,n);return v;}
static inline void aot_store(uint8_t*m,uint32_t a,uint64_t v,unsigned n,uint32_t null_size,int*err){if(a<null_size||(uint64_t)a+n>0x100000000ull){*err=1;return;}aot_exclusive_invalidate(a,n);memcpy(m+(uint64_t)a,&v,n);}
static inline void aot_branch(AcxiContext*c,uint32_t target,int exchange){if(exchange)c->cpsr=(c->cpsr&~32u)|((target&1u)<<5);c->regs[15]=target&((c->cpsr&32)?~1u:~3u);}
static inline float aot_f(AcxiContext*c,unsigned n){float v;memcpy(&v,&c->extregs[n],4);return v;}
static inline double aot_d(AcxiContext*c,unsigned n){double v;memcpy(&v,&c->extregs[n*2],8);return v;}
static inline void aot_sf(AcxiContext*c,unsigned n,float v){memcpy(&c->extregs[n],&v,4);}
static inline void aot_sd(AcxiContext*c,unsigned n,double v){memcpy(&c->extregs[n*2],&v,8);}
static inline uint64_t aot_dr(AcxiContext*c,unsigned n){uint64_t v;memcpy(&v,&c->extregs[n*2],8);return v;}
static inline void aot_sdr(AcxiContext*c,unsigned n,uint64_t v){memcpy(&c->extregs[n*2],&v,8);}
#include "acxi_softfloat.h"
static inline uint32_t aot_fp32(AcxiContext*c,int op,uint32_t x,uint32_t y){return acxi_vfp32(&c->fpscr,op,x,y);}
static inline uint64_t aot_fp64(AcxiContext*c,int op,uint64_t x,uint64_t y){return acxi_vfp64(&c->fpscr,op,x,y);}
static inline unsigned aot_clz(uint32_t x){if(!x)return 32;unsigned n=0;while(!(x&0x80000000u)){x<<=1;++n;}return n;}
#define LD8(a) ((uint32_t)aot_load(m,(uint32_t)(a),1,null_size,&err))
#define LD16(a) ((uint32_t)aot_load(m,(uint32_t)(a),2,null_size,&err))
#define LD32(a) ((uint32_t)aot_load(m,(uint32_t)(a),4,null_size,&err))
#define LD64(a) aot_load(m,(uint32_t)(a),8,null_size,&err)
#define ST8(a,v) aot_store(m,(uint32_t)(a),(uint8_t)(v),1,null_size,&err)
#define ST16(a,v) aot_store(m,(uint32_t)(a),(uint16_t)(v),2,null_size,&err)
#define ST32(a,v) aot_store(m,(uint32_t)(a),(uint32_t)(v),4,null_size,&err)
#define ST64(a,v) aot_store(m,(uint32_t)(a),(uint64_t)(v),8,null_size,&err)
#endif
'''

class Unsupported(Exception):pass

class Emitter:
    def __init__(self,image):self.image=image;self.unsupported=[]
    def regno(self,reg):
        n=self.ins.reg_name(reg)
        if n in ('sp','lr','pc'):return {'sp':13,'lr':14,'pc':15}[n]
        if n in ('sb','sl','fp','ip'):return {'sb':9,'sl':10,'fp':11,'ip':12}[n]
        if n.startswith('r') and n[1:].isdigit():return int(n[1:])
        raise Unsupported('integer register '+n)
    def r(self,reg):
        n=self.regno(reg)
        if n==15:return f'0x{self.pc+(4 if self.thumb else 8):08x}u'
        return f'c->regs[{n}]'
    def assign(self,reg,expr,exchange=False):
        n=self.regno(reg)
        return f'aot_branch(c,({expr}),{int(exchange)});' if n==15 else f'c->regs[{n}]=({expr});'
    def rawv(self,op):
        if op.type==arm.ARM_OP_REG:return self.r(op.reg)
        if op.type==arm.ARM_OP_IMM:return f'0x{op.imm&0xffffffff:x}u'
        raise Unsupported('scalar operand '+str(op.type))
    def shift(self,op):
        v=self.rawv(op);t=op.shift.type;n=op.shift.value
        if not t:return v
        typ={arm.ARM_SFT_LSL:0,arm.ARM_SFT_LSR:1,arm.ARM_SFT_ASR:2,arm.ARM_SFT_ROR:3,arm.ARM_SFT_RRX:3,
             arm.ARM_SFT_LSL_REG:0,arm.ARM_SFT_LSR_REG:1,arm.ARM_SFT_ASR_REG:2,arm.ARM_SFT_ROR_REG:3,arm.ARM_SFT_RRX_REG:3}.get(t)
        if typ is None:raise Unsupported('shift '+str(t))
        byreg=t>=arm.ARM_SFT_ASR_REG
        amount=self.r(n)+'&255u' if byreg else str(0 if t==arm.ARM_SFT_RRX else n)
        return f'aot_shift({v},{typ},{amount},{int(byreg)},(c->cpsr>>29)&1).v'
    def op2(self):
        if not self.thumb:
            raw=int.from_bytes(self.ins.bytes,'little')
            if raw&(1<<25):
                v=raw&255;rot=((raw>>8)&15)*2;v=((v>>rot)|(v<<(32-rot)))&0xffffffff if rot else v
                return [f'b=0x{v:x}u;',f'shc={v>>31}u;' if rot else 'shc=(c->cpsr>>29)&1;']
            rm=raw&15;typ=(raw>>5)&3;byreg=bool(raw&16);amount=f'(c->regs[{(raw>>8)&15}]&255u)' if byreg else str((raw>>7)&31)
            value=f'0x{self.pc+8:x}u' if rm==15 else f'c->regs[{rm}]'
            return [f'sh=aot_shift({value},{typ},{amount},{int(byreg)},(c->cpsr>>29)&1);b=sh.v;shc=sh.c;']
        op=self.ins.operands[-1]
        return [f'b={self.shift(op)};','shc=(c->cpsr>>29)&1;']
    def mem(self,index):
        op=self.ins.operands[index];base=self.r(op.mem.base) if op.mem.base else '0u'
        if self.thumb and op.mem.base==arm.ARM_REG_PC:base=f'0x{(self.pc+4)&~3:x}u'
        delta=str(op.mem.disp)
        if op.mem.index:
            ro=type('R',(),{'type':arm.ARM_OP_REG,'reg':op.mem.index,'shift':op.shift})()
            v=self.shift(ro);sgn='-' if op.mem.scale<0 or op.subtracted else '+';delta=f'({delta}{sgn}({v}))'
        # Capstone 5 reports post_index=False for A32 extra load/store
        # encodings (e.g. STRH r0,[r1],#2). The architectural P bit is reliable.
        post=bool(getattr(self.ins,'post_index',False)) if self.thumb else not bool(int.from_bytes(self.ins.bytes,'little')&(1<<24))
        if post:
            extra=self.ins.operands[index+1]
            delta=self.shift(extra)
            if extra.subtracted:delta=f'-({delta})'
        addr=base if post else f'({base}+({delta}))'
        before=[f'addr={addr};']
        after=[]
        if post or self.ins.writeback:after=[self.assign(op.mem.base,f'{base}+({delta})')]
        return before,after
    def freg(self,op):
        n=self.ins.reg_name(op.reg)
        if n.startswith('s') and n[1:].isdigit():return ('s',int(n[1:]))
        if n.startswith('d') and n[1:].isdigit():return ('d',int(n[1:]))
        raise Unsupported('float register '+n)
    def is_freg(self,op):
        if op.type!=arm.ARM_OP_REG:return False
        n=self.ins.reg_name(op.reg)
        return len(n)>1 and n[0] in 'sd' and n[1:].isdigit()
    def fraw(self,op):
        k,n=self.freg(op);return f'c->extregs[{n}]' if k=='s' else f'aot_dr(c,{n})'
    def fget(self,op):
        if op.type==arm.ARM_OP_FP:return repr(op.fp)
        if op.type==arm.ARM_OP_IMM:return str(op.imm)
        k,n=self.freg(op);return f'aot_{"f" if k=="s" else "d"}(c,{n})'
    def fset(self,op,expr,raw=False):
        k,n=self.freg(op)
        if raw:return f'c->extregs[{n}]=(uint32_t)({expr});' if k=='s' else f'aot_sdr(c,{n},({expr}));'
        return f'aot_s{"f" if k=="s" else "d"}(c,{n},({expr}));'
    def floatop(self,base,suffix):
        ops=self.ins.operands
        if base in ('vldr','vstr'):
            before,after=self.mem(1);kind,n=self.freg(ops[0]);width=32 if kind=='s' else 64
            op=self.fset(ops[0],f'LD{width}(addr)',True) if base=='vldr' else f'ST{width}(addr,{self.fraw(ops[0])});'
            return before+[op]+after
        if base in ('vpush','vpop','vldmia','vldmdb','vstmia','vstmdb','fldmiax','fstmdbx'):
            implicit=base in ('vpush','vpop');regs=ops if implicit else ops[1:];rn=13 if implicit else self.regno(ops[0].reg)
            size=sum(8 if self.freg(o)[0]=='d' else 4 for o in regs)
            extended=base in ('fldmiax','fstmdbx');size+=4 if extended else 0
            dec=base in ('vpush','vldmdb','vstmdb','fstmdbx');load=base in ('vpop','vldmia','vldmdb','fldmiax')
            lines=[f'base=c->regs[{rn}];addr=base{f"-{size}u" if dec else ""};']
            for op in regs:
                width=64 if self.freg(op)[0]=='d' else 32
                lines+=[self.fset(op,f'LD{width}(addr)',True) if load else f'ST{width}(addr,{self.fraw(op)});',f'addr+={width//8};']
            if self.ins.writeback or implicit:lines.append(f'c->regs[{rn}]=base{"-" if dec else "+"}{size}u;')
            return lines
        if base=='vmov':
            if len(ops)==2:
                d,s=ops
                if self.is_freg(d):
                    if self.is_freg(s):return [self.fset(d,self.fraw(s),True)]
                    if s.type==arm.ARM_OP_REG:return [self.fset(d,self.r(s.reg),True)]
                    return [self.fset(d,self.fget(s))]
                return [self.assign(d.reg,self.fraw(s))]
            if len(ops)==3:
                if self.is_freg(ops[0]):return [self.fset(ops[0],f'(uint64_t){self.r(ops[1].reg)}|((uint64_t){self.r(ops[2].reg)}<<32)',True)]
                return [f'wide={self.fraw(ops[2])};',self.assign(ops[0].reg,'(uint32_t)wide'),self.assign(ops[1].reg,'(uint32_t)(wide>>32)')]
        if base=='vmrs':
            if self.ins.reg_name(ops[0].reg)=='apsr_nzcv':return ['c->cpsr=(c->cpsr&0x0fffffffu)|(c->fpscr&0xf0000000u);']
            return [self.assign(ops[0].reg,'c->fpscr')]
        if base=='vmsr':return [f'c->fpscr={self.r(ops[1].reg)};']
        if base in ('vcmp','vcmpe'):
            is64=int(self.freg(ops[0])[0]=='d');right=self.fraw(ops[1]) if self.is_freg(ops[1]) else '0'
            return [f'acxi_vfp_compare(&c->fpscr,{self.fraw(ops[0])},{right},{is64},{int(base=="vcmpe")});']
        if base in ('vcvt','vcvtr'):
            ds,ss=suffix.split('.')[:2];raw=self.fraw(ops[1])
            if ss in ('s32','u32'):expr=f'acxi_int_to_vfp(&c->fpscr,{raw},{int(ss=="u32")},{int(ds=="f64")})'
            elif ds in ('s32','u32'):expr=f'acxi_vfp_to_int(&c->fpscr,{raw},{int(ss=="f64")},{int(ds=="u32")},{int(base=="vcvt")})'
            else:expr=f'acxi_vfp_convert(&c->fpscr,{raw},{int(ss=="f64")})'
            return [self.fset(ops[0],expr,True)]
        if base in ('vadd','vsub','vmul','vdiv','vmla','vmls','vnmla','vnmls','vnmul','vneg','vabs','vsqrt'):
            a=self.fraw(ops[1]);b=self.fraw(ops[2]) if len(ops)>2 else '0';old=self.fraw(ops[0]);width=32 if suffix=='f32' else 64
            sign='0x80000000u' if width==32 else 'UINT64_C(0x8000000000000000)'
            fn=f'aot_fp{width}'
            if base=='vneg':expr=f'({a})^{sign}'
            elif base=='vabs':expr=f'({a})&~{sign}'
            elif base=='vsqrt':expr=f'{fn}(c,4,{a},0)'
            elif base in ('vadd','vsub','vmul','vdiv'):expr=f'{fn}(c,{["vadd","vsub","vmul","vdiv"].index(base)},{a},{b})'
            elif base=='vnmul':expr=f'{fn}(c,2,{a},{b})^{sign}'
            else:
                prod=f'{fn}(c,2,{a},{b})'
                if base=='vmla':expr=f'{fn}(c,0,{old},{prod})'
                elif base=='vmls':expr=f'{fn}(c,0,{old},({prod})^{sign})'
                elif base=='vnmla':expr=f'{fn}(c,0,({old})^{sign},({prod})^{sign})'
                else:expr=f'{fn}(c,0,({old})^{sign},{prod})'
            return [self.fset(ops[0],expr,True)]
        raise Unsupported(base+'.'+suffix)
    def translate(self,ins,thumb):
        self.ins=ins;self.thumb=thumb;self.pc=ins.address+self.image.slide
        parts=ins.mnemonic.split('.');base=parts[0];suffix='.'.join(parts[1:])
        cond=ins.cc-1 if ins.cc in COND else 14
        if ins.cc in COND:
            endings={COND[ins.cc]}
            if ins.cc==3:endings.add('cs')
            if ins.cc==4:endings.add('cc')
            for e in endings:
                if base.endswith(e):base=base[:-2];break
        if base.endswith('s') and base[:-1] in ('mov','mvn','add','adc','sub','sbc','rsb','rsc','and','orr','eor','bic','lsl','lsr','asr','ror','rrx','mul'):base=base[:-1]
        # A32's S bit is authoritative. Capstone marks some non-S ADC
        # instructions as update_flags=True, which silently corrupts NZCV.
        ops=ins.operands;flags=int(ins.update_flags) if thumb else (int.from_bytes(ins.bytes,'little')>>20)&1
        if base.startswith('v') or base in ('fldmiax','fstmdbx'):body=self.floatop(base,suffix)
        elif base in ('b','bl','blx','bx'):
            call=base in ('bl','blx');exchange=base in ('bx','blx')
            target=f'0x{(ops[0].imm+self.image.slide)&0xffffffff:x}u' if ops[0].type==arm.ARM_OP_IMM else self.r(ops[0].reg)
            if base=='blx' and ops[0].type==arm.ARM_OP_IMM:target=f'({target}|{int(not thumb)}u)'
            body=[f'v={target};']
            if call:body.append(f'c->regs[14]=0x{(self.pc+ins.size)|int(thumb):x}u;')
            body.append(f'aot_branch(c,v,{int(exchange)});')
        elif base in ('cbz','cbnz'):
            target=ops[1].imm+self.image.slide
            body=[f'if({self.r(ops[0].reg)}{"==" if base=="cbz" else "!="}0)c->regs[15]=0x{target:x}u;']
        elif base in ('add','adc','sub','sbc','rsb','rsc','and','orr','eor','bic','mov','mvn','cmp','cmn','tst','teq','lsl','lsr','asr','ror','rrx'):
            if not thumb:body=self.op2()
            else:body=[f'b={self.shift(ops[-1])};shc=(c->cpsr>>29)&1;']
            comp=base in ('cmp','cmn','tst','teq');unary=base in ('mov','mvn','lsl','lsr','asr','ror','rrx')
            if not unary:body.append(f'a={self.r(ops[0 if comp else 1 if len(ops)>2 else 0].reg)};')
            if thumb and base in ('lsl','lsr','asr','ror','rrx'):
                typ={'lsl':0,'lsr':1,'asr':2,'ror':3,'rrx':3}[base];src=ops[1] if len(ops)>2 else ops[0];amount=self.rawv(ops[-1]) if base!='rrx' else '0'
                body=[f'sh=aot_shift({self.rawv(src)},{typ},{amount},{int(ops[-1].type==arm.ARM_OP_REG)},(c->cpsr>>29)&1);b=sh.v;shc=sh.c;']
            if base in ('add','adc','cmn'):expr=f'aot_add(c,a,b,{"(c->cpsr>>29)&1" if base=="adc" else "0"},{1 if comp else flags})'
            elif base in ('sub','sbc','cmp'):expr=f'aot_add(c,a,~b,{"(c->cpsr>>29)&1" if base=="sbc" else "1"},{1 if comp else flags})'
            elif base in ('rsb','rsc'):expr=f'aot_add(c,b,~a,{"(c->cpsr>>29)&1" if base=="rsc" else "1"},{flags})'
            else:
                expr={'and':'a&b','tst':'a&b','orr':'a|b','eor':'a^b','teq':'a^b','bic':'a&~b','mov':'b','mvn':'~b','lsl':'b','lsr':'b','asr':'b','ror':'b','rrx':'b'}[base]
            body.append(f'v={expr};')
            if base not in ('add','adc','sub','sbc','rsb','rsc','cmp','cmn') and (flags or comp):body.append('aot_logic(c,v,shc);')
            if not comp:body.append(self.assign(ops[0].reg,'v'))
        elif base in ('ldr','ldrb','ldrh','ldrsb','ldrsh','str','strb','strh','ldrd','strd','ldrex','strex','swp','swpb'):
            mi=next(i for i,o in enumerate(ops) if o.type==arm.ARM_OP_MEM);before,after=self.mem(mi);body=before
            if base=='ldrex':body.append(self.assign(ops[0].reg,'aot_ldrex(c,m,addr,null_size,&err)'))
            elif base=='strex':body.append(self.assign(ops[0].reg,f'aot_strex(c,m,addr,{self.r(ops[1].reg)},null_size,&err)'))
            elif base in ('swp','swpb'):
                bits=8 if base=='swpb' else 32;body+=[f'v=LD{bits}(addr);',f'ST{bits}(addr,{self.r(ops[1].reg)});',self.assign(ops[0].reg,'v')]
            elif base.startswith('ldr'):
                bits=8 if base.endswith('b') else 16 if base.endswith('h') else 32
                cast='(uint32_t)(int32_t)(int8_t)' if base=='ldrsb' else '(uint32_t)(int32_t)(int16_t)' if base=='ldrsh' else ''
                body.append(f'v={cast}LD{bits}(addr);')
                if base=='ldrd':body+=[f'b=LD32(addr+4u);']
                body+=after+[self.assign(ops[0].reg,'v',True)]
                if base=='ldrd':body.append(self.assign(ops[1].reg,'b'))
                after=[]
            else:
                bits=8 if base.endswith('b') else 16 if base.endswith('h') else 32
                body.append(f'ST{bits}(addr,{self.r(ops[0].reg)});')
                if base=='strd':body.append(f'ST32(addr+4u,{self.r(ops[1].reg)});')
            body+=after
        elif base in ('push','pop','ldm','ldmia','ldmib','ldmda','ldmdb','stm','stmia','stmib','stmda','stmdb'):
            implicit=base in ('push','pop');rn=13 if implicit else self.regno(ops[0].reg);regs=ops if implicit else ops[1:];n=len(regs);load=base=='pop' or base.startswith('ldm')
            mode='db' if base=='push' else 'ia' if implicit or base in ('ldm','stm') else base[-2:]
            delta={'ia':0,'ib':4,'da':-4*(n-1),'db':-4*n}[mode]
            body=[f'base=c->regs[{rn}];addr=base+({delta});']
            if (ins.writeback or implicit) and load:body.append(f'c->regs[{rn}]=base{"-" if mode[0]=="d" else "+"}{4*n}u;')
            for o in regs:
                if load:body+=[f'v=LD32(addr);',self.assign(o.reg,'v',True)]
                else:body.append(f'ST32(addr,{self.r(o.reg)});')
                body.append('addr+=4u;')
            if (ins.writeback or implicit) and not load:body.append(f'c->regs[{rn}]=base{"-" if mode[0]=="d" else "+"}{4*n}u;')
        elif base in ('mul','mla','mls','umull','smull','umlal','smlal','smulbb','smulbt','smultb','smultt'):
            if base in ('umull','smull','umlal','smlal'):
                signed=base.startswith('s');cast='(int64_t)(int32_t)' if signed else '(uint64_t)';a=self.r(ops[2].reg);b=self.r(ops[3].reg)
                body=[f'wide=(uint64_t)({cast}({a})*{cast}({b}));']
                if base.endswith('lal'):body.append(f'wide+=((uint64_t){self.r(ops[1].reg)}<<32)|{self.r(ops[0].reg)};')
                body += [self.assign(ops[0].reg,'(uint32_t)wide'),self.assign(ops[1].reg,'(uint32_t)(wide>>32)')]
                if flags:body.append('c->cpsr=(c->cpsr&0x3fffffffu)|((uint32_t)(wide>>32)&0x80000000u)|(wide?0:0x40000000u);')
            else:
                a=self.r(ops[1].reg);b=self.r(ops[2].reg)
                if base.startswith('smul'):
                    a=f'(int32_t)(int16_t)({a}{">>16" if base[-2]=="t" else ""})';b=f'(int32_t)(int16_t)({b}{">>16" if base[-1]=="t" else ""})'
                expr=f'({a})*({b})'
                if base=='mla':expr+=f'+{self.r(ops[3].reg)}'
                if base=='mls':expr=f'{self.r(ops[3].reg)}-({expr})'
                body=[f'v={expr};',self.assign(ops[0].reg,'v')]
                if flags:body.append('aot_nz(c,v);')
        elif base in ('sxtb','sxth','uxtb','uxth','sxtab','sxtah','uxtab','uxtah'):
            val=self.shift(ops[-1]);signed=base.startswith('s');bits=8 if base.endswith('b') else 16;cast=f'({"int" if signed else "uint"}{bits}_t)'
            expr=f'(uint32_t)(int32_t){cast}({val})' if signed else f'(uint32_t){cast}({val})'
            if 'a' in base[3:]:expr=f'{self.r(ops[1].reg)}+({expr})'
            body=[self.assign(ops[0].reg,expr)]
        elif base in ('clz','rev','rev16','revsh'):
            v=self.r(ops[1].reg)
            expr=f'aot_clz({v})' if base=='clz' else f'(({v}&255u)<<24)|(({v}&65280u)<<8)|(({v}>>8)&65280u)|({v}>>24)' if base=='rev' else f'(({v}&0x00ff00ffu)<<8)|(({v}>>8)&0x00ff00ffu)' if base=='rev16' else f'(uint32_t)(int32_t)(int16_t)((({v}&255u)<<8)|(({v}>>8)&255u))'
            body=[self.assign(ops[0].reg,expr)]
        elif base=='mrs':body=[self.assign(ops[0].reg,'c->cpsr')]
        elif base=='msr':body=[f'c->cpsr=(c->cpsr&0x0fffffffu)|({self.r(ops[-1].reg)}&0xf0000000u);']
        elif base in ('nop','pld','yield','dmb','dsb','isb'):body=['/* architectural hint/barrier; guest threads are cooperatively scheduled */']
        elif base=='mcr':
            raw=int.from_bytes(ins.bytes,'little')
            if raw&0x0fff0fff in (0x0e070fba,0x0e070f9a,0x0e070f95):body=['/* ARMv6 data memory/synchronization barrier */']
            else:raise Unsupported('coprocessor write '+ins.op_str)
        elif base=='svc':body=[f'return {ops[0].imm&0xffffff};']
        elif base=='bkpt':body=['return -4;']
        else:raise Unsupported(base+' '+ins.op_str)
        if cond!=14:body=['if(aot_cond(c->cpsr,'+str(cond)+')){']+body+['}']
        return body

def generate(ipa, runtime, output, full=False):
    global OUT
    OUT=Path(output).resolve()
    OUT.mkdir(parents=True,exist_ok=True)
    bridge=ROOT/'trampoline.c'
    shutil.copyfile(bridge,OUT/'trampoline.c')
    (OUT/'acxi_aot.h').write_text(HEADER)
    runtime=Path(runtime).resolve()
    dylibs=runtime/'touchHLE_dylibs'
    images=[Image(ipa),Image(dylibs/'libgcc_s.1.dylib',0x30000000),Image(dylibs/'libstdc++.6.0.9.dylib',0x3748a000)]
    expected=['12cb706225c81c2707b1f876268d0ea825b33ffe','bd00bca7a28e04403d227cfd3fba6c1e4793dc5c','598bbdf6c26e5355612acd784755d9ff86fc53d0']
    for image,digest in zip(images,expected):
        if hashlib.sha1(image.data).hexdigest()!=digest:
            raise ValueError('Input image identity mismatch: '+image.name)
    pages=collections.defaultdict(list);stats=[];problems=[]
    for im in images:
        im.discover();r=im.report();cfg=set(im.instructions);em=Emitter(im);hist=collections.Counter();cfg_bad=0
        if full:
            md=cs.Cs(cs.CS_ARCH_ARM,cs.CS_MODE_ARM|cs.CS_MODE_LITTLE_ENDIAN);md.detail=True
            for sec in im.code:
                for pc in range((sec['addr']+3)&~3,sec['addr']+sec['size']-3,4):
                    if (pc,False) in im.instructions:continue
                    ins=next(md.disasm(im.read(pc,4),pc,count=1),None)
                    if ins:im.instructions[(pc,False)]=ins
        for (pc,t),ins in sorted(im.instructions.items()):
            address=pc+im.slide;key=address|(1 if t else 0)
            lines=[f'case 0x{key:x}u: /* {ins.mnemonic} {ins.op_str} */',f'c->regs[15]=0x{address+ins.size:x}u;']
            try:lines+=em.translate(ins,t)
            except (Unsupported,IndexError,ValueError) as e:
                problems.append({'module':im.name,'address':hex(address),'instruction':ins.mnemonic+' '+ins.op_str,'reason':str(e),'cfg_reachable':(pc,t) in cfg});hist[ins.mnemonic]+=1;cfg_bad+=int((pc,t) in cfg)
                lines+=['return -3;']
            lines+=['return err?-2:AOT_CONTINUE;']
            pages[address>>12].append('\n'.join(lines))
        r['generated_instruction_count']=len(im.instructions);r['unsupported_count']=sum(hist.values());r['cfg_unsupported_count']=cfg_bad;r['unsupported_mnemonics']=dict(hist);stats.append(r)
        print(json.dumps({'module':im.name,'instructions':len(im.instructions),'unsupported':sum(hist.values()),'unsupported_mnemonics':dict(hist)}),flush=True)
    declarations=[];dispatch=[];sources=['runtime.c','trampoline.c']
    for page,entries in sorted(pages.items()):
        name=f'page_{page:05x}';file=name+'.c';sources.append(file)
        declarations.append(f'int32_t {name}(AcxiContext*,uint8_t*,uint32_t);')
        dispatch.append(f'case 0x{page:x}u: result={name}(c,m,null_size);break;')
        code='#include "acxi_aot.h"\n'+f'int32_t {name}(AcxiContext*c,uint8_t*m,uint32_t null_size){{\nuint32_t a,b,v,shc,addr,base;uint64_t wide;AotShift sh;int err=0;\nswitch(c->regs[15]|((c->cpsr>>5)&1u)){{\n'+'\n'.join(entries)+'\ndefault:return -5;\n}\n}\n'
        (OUT/file).write_text(code)
    stub_ranges=[(s['addr']+im.slide,s['addr']+im.slide+s['size']) for im in images for s in im.stub_sections]
    stub_predicate='||'.join(f'(pc>=0x{lo:x}u&&pc<0x{hi:x}u)' for lo,hi in stub_ranges) or 'false'
    code='#include "acxi_aot.h"\n'+f'bool acxi_aot_is_stub(uint32_t pc){{return {stub_predicate};}}\n'+'\n'.join(declarations)+r'''
static _Thread_local uint32_t exclusive_address,exclusive_value;
static _Thread_local bool exclusive_valid;
void acxi_aot_clear_exclusive(void){exclusive_valid=false;}
void aot_exclusive_invalidate(uint32_t a,unsigned n){if(exclusive_valid&&(uint64_t)a<((uint64_t)exclusive_address+4)&&(uint64_t)exclusive_address<((uint64_t)a+n))exclusive_valid=false;}
uint32_t aot_ldrex(AcxiContext*c,uint8_t*m,uint32_t a,uint32_t null_size,int*err){(void)c;uint32_t v=(uint32_t)aot_load(m,a,4,null_size,err);exclusive_valid=!*err;exclusive_address=a;exclusive_value=v;return v;}
uint32_t aot_strex(AcxiContext*c,uint8_t*m,uint32_t a,uint32_t v,uint32_t null_size,int*err){(void)c;bool ok=exclusive_valid&&a==exclusive_address;exclusive_valid=false;if(!ok)return 1;uint32_t current=(uint32_t)aot_load(m,a,4,null_size,err);if(*err||current!=exclusive_value)return 1;aot_store(m,a,v,4,null_size,err);return *err?1:0;}
int32_t acxi_aot_run(AcxiContext*c,uint8_t*m,uint32_t null_size,uint64_t*ticks,bool single_step){
    uint64_t remaining=single_step?1:(ticks?*ticks:1);int32_t result=-1;
    while(remaining){
        switch(c->regs[15]>>12){
'''+ '\n'.join(dispatch)+r'''
        default:result=-5;break;
        }
        if(result==-5)result=acxi_aot_trampoline(c,m,null_size);
        --remaining;
        if(result!=AOT_CONTINUE&&result!=-6){if(ticks)*ticks=remaining;return result;}
    }
    if(ticks)*ticks=remaining;
    return -1;
}
'''
    (OUT/'runtime.c').write_text(code)
    from prepare_softfloat import prepare
    sources += prepare(OUT,runtime)
    (OUT/'sources.txt').write_text('\n'.join(sources)+'\n')
    (OUT/'coverage.json').write_text(json.dumps(stats,indent=2)+'\n')
    (OUT/'unsupported.json').write_text(json.dumps(problems,indent=2)+'\n')
    print('Generated',len(pages),'translation pages and',len(problems),'explicit unsupported traps',flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output',type=Path)
    parser.add_argument('--ipa',required=True,type=Path)
    parser.add_argument('--runtime',type=Path,default=ROOT.parents[1]/'runtime')
    parser.add_argument('--full',action='store_true')
    args=parser.parse_args()
    generate(args.ipa,args.runtime,args.output,args.full)
