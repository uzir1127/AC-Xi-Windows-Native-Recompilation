# ACXi port tools, 2026-10-29. SPDX-License-Identifier: MPL-2.0
"""Add narrow, reproducible native port hooks to verified generated code.

Original IPA and machine-code bytes are untouched. Selected C translation pages
get native hooks; original generated pages are retained for comparison.
"""
from pathlib import Path
import argparse,hashlib,json,re
root=Path(__file__).resolve().parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('generated',type=Path)
parser.add_argument('--backup',type=Path)
args=parser.parse_args()
generated=args.generated.resolve()
backup=args.backup.resolve() if args.backup else generated.parent/'hook-originals'
backup.mkdir(parents=True,exist_ok=True)
old_manifest=json.loads((backup/'manifest.json').read_text()) if (backup/'manifest.json').exists() else []
old_hashes={item['page']:item.get('generated_sha256') for item in old_manifest}
hooks={
    # PlayerMove normal simulation path, after weapons and physics. STATE_PL
    # +0xbc is the original afterburner blend; +0x21c bit 3 is stall.
    'page_00017.c':('''extern void acxi_rumble_frame(unsigned int,unsigned int);
extern void acxi_rumble_event(unsigned int,unsigned int);
''',0x172f4,'acxi_rumble_frame(LD32(0x125d10u+0x21cu),LD32(0x125d10u+0xbcu));'),
    # This path is reached only after WeaponFire, after ammo/cooldown checks.
    'page_0001b.c':('extern void acxi_rumble_event(unsigned int,unsigned int);\n',0x1bc48,'acxi_rumble_event(2u,0x3f800000u);'),
    # Player gun burst after SetPlayerGun, not merely the held gun action.
    'page_0001c.c':('extern void acxi_rumble_event(unsigned int,unsigned int);\n',0x1c458,'acxi_rumble_event(1u,0x3f800000u);'),
    # CameraMove phase 1 already has an analog look path, original angle
    # limits, quarter-step smoothing and view-specific HUD/Behind geometry.
    # Connect host right-stick axes only for those normal flight views.
    'page_00014.c':('''extern unsigned long long acxi_native_look_axes(void);
static unsigned int acxi_look_owner,acxi_look_active;
static unsigned long long acxi_look_sample;
''',0x145b4,'''{
    unsigned long long sample=acxi_native_look_axes();
    uint32_t camera=c->regs[4],state=LD32(0xfa0c0u);
    acxi_look_active=0;
    if(camera>=null_size && (uint64_t)camera+0x2cu<=0x100000000ull &&
       state>=null_size && (uint64_t)state+0x143u<=0x100000000ull){
        uint32_t kind=LD8(camera+0x18u),view=LD8(state+0x142u);
        if((sample&(1ull<<32)) && (view==0u || view==2u) && (kind==1u || kind==4u)){
            uint32_t pitch=LD32(camera+0x1cu)&0x7fffffffu,yaw=LD32(camera+0x20u)&0x7fffffffu;
            if((uint32_t)sample || (acxi_look_owner==camera && (pitch>=0x3a83126fu || yaw>=0x3a83126fu))){
                acxi_look_owner=camera;acxi_look_active=1;acxi_look_sample=sample;
                ST8(camera+0x18u,4u);
                /* Original latch: free look remains available while steering. */
                ST16(camera+0x14u,LD16(camera+0x14u)|4u);
            }else if(acxi_look_owner==camera){
                ST32(camera+0x1cu,0u);ST32(camera+0x20u,0u);acxi_look_owner=0;
            }
        }else if(acxi_look_owner==camera){
            ST32(camera+0x1cu,0u);ST32(camera+0x20u,0u);
            if(kind==4u)ST8(camera+0x18u,1u);
            acxi_look_owner=0;
        }
    }else{acxi_look_owner=0;}
}'''),
    # Merge host flight actions before the original pause/in-game restrictions
    # and held/pressed/released bookkeeping. No five-finger limit or hidden UI
    # hit-test dependency; the original game still implements every action.
    'page_00052.c':('extern unsigned int acxi_native_flight_buttons(void);\n',0x52298,'c->regs[5]|=acxi_native_flight_buttons();'),
    # The mobile PadIn path omits ViewSelect, although the original player
    # action dispatcher still calls it once per update. Feed its original
    # pressed edge to that action; retain the game's camera transition code.
    'page_0001e.c':('extern unsigned int acxi_native_flight_buttons(void);\n',0x1e284,'''if(acxi_native_flight_buttons()&256u){
    uint32_t mode=LD32(0x10176cu);
    if(mode>=null_size && (uint64_t)mode+0x6cu<=0x100000000ull)
        c->regs[2]|=(LD32(mode+0x68u)&256u)!=0;
}'''),
    # CGameState::InitializeSaveData common exit: r4 is the initialized state,
    # after either validated save loading or new-save defaults. These are the
    # 14 airframe IDs in nsDLC::s_ProductIDTable, not the five base/reward planes.
    # Bit 1 is ownership (the original purchase completion at 0xaa0bc/0xaa0dc).
    # OR only that bit; preserve progression, other flags and save validation.
    'page_00023.c':('',0x230a4,'''{
    static const unsigned char paid_airframes[] = {1,15,16,17,39,20,19,18,23,22,7,2,8,4};
    uint32_t state=c->regs[4];
    if(state<null_size || (uint64_t)state+0xcc0u>0x100000000ull)
        return aot_page_exit(budget,remaining,-2);
    for(unsigned int i=0;i<sizeof(paid_airframes);++i){
        uint32_t flag=state+0x306u+2u*paid_airframes[i];
        ST16(flag,LD16(flag)|2u);
    }
    /* ace_level_pack01_otp: the original additional-mission ownership bit. */
    ST32(state+0x300u,LD32(state+0x300u)|2u);
}'''),
    'page_00058.c':('extern int acxi_draw_touch_buttons(void);\n',0x58514,'if(!acxi_draw_touch_buttons()){aot_branch(c,c->regs[14],1);goto dispatch;}'),
    'page_00054.c':('extern int acxi_draw_phone_hint(void);\n',0x54e18,'if(!acxi_draw_phone_hint()){aot_branch(c,c->regs[14],1);goto dispatch;}'),
    # Observe after UpdateTouchButton assembles its global OK/Back buttons.
    # r4 is the menu object, r10 its source rectangle table at this point.
    'page_0008b.c':('extern void acxi_observe_menu_table(const unsigned char*,unsigned int,unsigned int,unsigned int,unsigned int);\n',0x8bea0,'if(c->regs[4]>=4096u && (uint64_t)c->regs[4]+0x70u<=0x100000000ull){uint32_t n=LD32(c->regs[4]+0x64u),f=LD32(c->regs[4]+0x2cu),owner=LD32(c->regs[4]),phase=0;if(owner==0xfcf84u || owner==0xfd1c4u)phase=LD32(c->regs[4]+0x6cu);if(n<=128u && (!n || c->regs[10]>=4096u) && (uint64_t)c->regs[10]+(uint64_t)n*20u<=0x100000000ull)acxi_observe_menu_table(n?m+c->regs[10]:NULL,n,f,owner,phase);}'),
}
extra_hooks={
    'page_00017.c':[(0x175b0,'''{
    uint32_t player=c->regs[4];
    if(player>=null_size && (uint64_t)player+0xb2u<=0x100000000ull){
        int before=(short)c->regs[8],after=(short)LD16(player+0xb0u);
        int maximum=(short)LD16(0x125d10u+0x40u);
        if(before>after && before>0 && after>=0 && maximum>0){
            union {float f;unsigned int u;} damage;
            damage.f=(float)(before-after)/(float)maximum;
            acxi_rumble_event(3u,damage.u);
        }
    }
}''')],
    # One-time player destruction branch, immediately after explosion sound.
    'page_0001b.c':[(0x1bf60,'acxi_rumble_event(4u,0x3f800000u);')],
    'page_00014.c':[
        (0x14630,'''if(acxi_look_active){
    union {float f;unsigned int u;} x,y;
    x.f=(float)(short)(acxi_look_sample&65535u)/32767.0f;
    y.f=-(float)(short)((acxi_look_sample>>16)&65535u)/32767.0f;
    c->extregs[17]=x.u;c->extregs[16]=y.u;
}'''),
    ],
    # ACXi's own pause menu offers HUD (0) and Behind (2). The inherited
    # three-view action still includes an unsupported cockpit (1).
    'page_0001e.c':[(0x1e2b4,'if((acxi_native_flight_buttons()&256u) && c->regs[5]==1u)c->regs[5]=2u;')],
}
manifest=[]
for name,(decl,pc,code) in hooks.items():
    path=generated/name
    saved=backup/name
    current=path.read_text()
    if not saved.exists():
        if 'ACXI_NATIVE_PORT_HOOK' in current:raise RuntimeError('Missing original page backup')
        saved.write_text(current)
    original=saved.read_text()
    pattern=rf'(L_{pc:x}: /\*[^\n]*\*/\nif\(!remaining\)return aot_page_exit\(budget,remaining,-1\);\n--remaining;\n)'
    modified,n=re.subn(pattern,lambda m:m[1]+'/* ACXI_NATIVE_PORT_HOOK */\n'+code+'\n',original)
    if n!=1:raise RuntimeError(f'Entry changed: {name} {pc:x}')
    for extra_pc,extra_code in extra_hooks.get(name,[]):
        extra_pattern=rf'(L_{extra_pc:x}: /\*[^\n]*\*/\nif\(!remaining\)return aot_page_exit\(budget,remaining,-1\);\n--remaining;\n)'
        modified,n=re.subn(extra_pattern,lambda m:m[1]+'/* ACXI_NATIVE_PORT_HOOK */\n'+extra_code+'\n',modified)
        if n!=1:raise RuntimeError(f'Entry changed: {name} {extra_pc:x}')
    modified=decl+modified
    allowed=[original,modified]
    if name=='page_0008b.c':
        old_decl='extern void acxi_observe_menu_table(const unsigned char*,unsigned int);\n'
        old_code='if(c->regs[1]>=4096u && c->regs[2]<=128u && (uint64_t)c->regs[1]+(uint64_t)c->regs[2]*20u<=0x100000000ull)acxi_observe_menu_table(m+c->regs[1],c->regs[2]);'
        old_pattern=r'(L_8bd54: /\*[^\n]*\*/\nif\(!remaining\)return aot_page_exit\(budget,remaining,-1\);\n--remaining;\n)'
        allowed.append(old_decl+re.sub(old_pattern,lambda m:m[1]+'/* ACXI_NATIVE_PORT_HOOK */\n'+old_code+'\n',original))
    if current not in allowed and hashlib.sha256(current.encode()).hexdigest()!=old_hashes.get(name):raise RuntimeError(f'Unexpected local changes in {path}')
    if current!=modified:path.write_text(modified)
    manifest.append({'page':name,'original_sha256':hashlib.sha256(original.encode()).hexdigest(),'generated_sha256':hashlib.sha256(modified.encode()).hexdigest(),'hook_entry':hex(pc),'hook':code,'additional_hooks':[(hex(p),c) for p,c in extra_hooks.get(name,[])]})
(backup/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Native port hooks checked: flight buttons, view action, right-stick camera and vibration events; paid aircraft/mission ownership; menu rectangles; touch-button drawing; phone hint drawing')
