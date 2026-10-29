// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Optional, content-addressed replacements for this game's static assets.
//! Only known original uploads match. Guest dimensions, UVs and game memory
//! remain untouched. Inference is an offline pack-building step.
use crate::gles::{gles11_raw as gl, GLES};
use crate::image::Image;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug)]
struct Entry { width: i32, height: i32, digest: String, name: String }
struct Pack { enabled: bool, root: PathBuf, entries: HashMap<String,Entry> }
static PACK: OnceLock<Pack> = OnceLock::new();

fn valid_hex(s: &str) -> bool { s.len()==40 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) }
fn parse_index(text: &str) -> HashMap<String,Entry> {
    text.lines().filter_map(|line| {
        let f: Vec<_> = line.split('\t').collect();
        if f.len()!=5 { return None; }
        let k:Vec<_>=f[0].split('-').collect();
        if k.len()!=4 || !valid_hex(k[3]) || !valid_hex(f[3]) { return None; }
        let (ow,oh)=k[0].split_once('x')?;
        let (ow,oh)=(ow.parse::<i32>().ok()?,oh.parse::<i32>().ok()?);
        if ow<8 || oh<8 || !k[1..3].iter().all(|s|s.len()==4&&s.bytes().all(|b|b.is_ascii_hexdigit())) {return None;}
        let (width,height)=(f[1].parse::<i32>().ok()?,f[2].parse::<i32>().ok()?);
        if width>4096 || height>4096 || width<=ow || height<=oh || width%ow!=0 || height%oh!=0 || width/ow!=height/oh || !matches!(width/ow,2|4) { return None; }
        Some((f[0].to_owned(), Entry {width,height,digest:f[3].to_owned(),name:f[4].to_owned()}))
    }).collect()
}
pub fn initialize(enabled: bool) {
    PACK.get_or_init(|| {
        let root=PathBuf::from("hd-assets");
        let entries=if enabled { std::fs::read_to_string(root.join("textures.tsv")).map(|s|parse_index(&s)).unwrap_or_default() } else {HashMap::new()};
        log!("HD assets: {} ({} indexed textures)",if enabled {"enabled"} else {"original"},entries.len());
        Pack {enabled,root,entries}
    });
}
pub fn enabled() -> bool { PACK.get().is_some_and(|p|p.enabled) }
fn key(width:i32,height:i32,format:u32,kind:u32,bytes:&[u8])->String {
    format!("{width}x{height}-{format:04x}-{kind:04x}-{:x}",Sha1::digest(bytes))
}

/// Called only for full, level-zero guest texture uploads. ACXi imports no
/// glTexSubImage2D, glCopyTex* or mip-generation API; multi-mip PVRs are excluded
/// from the pack. The source assets in the IPA are immutable.
#[allow(clippy::too_many_arguments)]
pub fn upload(gles:&mut dyn GLES,target:u32,level:i32,width:i32,height:i32,border:i32,format:u32,kind:u32,bytes:&[u8])->bool {
    let Some(pack)=PACK.get().filter(|p|p.enabled&&!p.entries.is_empty()) else {return false;};
    if target!=gl::TEXTURE_2D || level!=0 || border!=0 || width<8 || height<8 { return false; }
    let key=key(width,height,format,kind,bytes);
    let Some(entry)=pack.entries.get(&key) else { return false; };
    let result=(||->Result<Image,String> {
        let file=std::fs::read(pack.root.join("textures").join(format!("{key}.png"))).map_err(|e|e.to_string())?;
        if file.len()>80*1024*1024 || format!("{:x}",Sha1::digest(&file))!=entry.digest { return Err("PNG size or checksum mismatch".into()); }
        let mut limit=0;
        unsafe { gles.GetIntegerv(gl::MAX_TEXTURE_SIZE,&mut limit); }
        if entry.width>limit || entry.height>limit { return Err(format!("GPU texture limit {limit}")); }
        // Check PNG dimensions before decoding; do not allocate arbitrary sizes.
        if file.len()<24 || &file[..8]!=b"\x89PNG\r\n\x1a\n" || u32::from_be_bytes(file[16..20].try_into().unwrap())!=entry.width as u32 || u32::from_be_bytes(file[20..24].try_into().unwrap())!=entry.height as u32 {return Err("Unexpected PNG dimensions".into());}
        let image=Image::from_bytes_straight(&file)?;
        if image.dimensions()!=(entry.width as u32,entry.height as u32) {return Err("Decoded size mismatch".into());}
        Ok(image)
    })();
    let image=match result {Ok(image)=>image,Err(e)=>{log!("HD asset fallback: {}: {e}",entry.name);return false;}};
    unsafe { gles.TexImage2D(target,level,gl::RGBA as _,entry.width,entry.height,border,gl::RGBA,gl::UNSIGNED_BYTE,image.pixels().as_ptr().cast()); }
    log!("HD texture: {} {}x{} -> {}x{}",entry.name,width,height,entry.width,entry.height);
    true
}

/// The enhanced movie has the same timestamps and copied AAC stream. If the
/// optional file is absent, corrupt or fails to decode, callers use the original.
pub fn movie(original:&[u8])->Option<Vec<u8>> {
    let pack=PACK.get().filter(|p|p.enabled)?;
    let manifest=std::fs::read_to_string(pack.root.join("movie.tsv")).ok()?;
    let f:Vec<_>=manifest.trim().split('\t').collect();
    if f.len()!=2 || !valid_hex(f[0]) || !valid_hex(f[1]) || format!("{:x}",Sha1::digest(original))!=f[0] {return None;}
    let path=pack.root.join("ACEOP.mp4");
    if std::fs::metadata(&path).ok()?.len()>256*1024*1024 {return None;}
    let bytes=std::fs::read(path).ok()?;
    if format!("{:x}",Sha1::digest(&bytes))!=f[1] {log!("HD movie checksum mismatch; using original");return None;}
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_rejects_paths_wrong_scale_and_oversized_images() {
        let h="0123456789abcdef0123456789abcdef01234567";
        let row=format!("256x256-1908-8033-{h}\t1024\t1024\t{h}\tfont.pvr");
        assert_eq!(parse_index(&row).len(),1);
        for bad in [row.replace("1024\t1024","1024\t512"),row.replace("1024\t1024","8192\t8192"),row.replace("256x256","../oops"),row.replace(h,"../../../escape")] {assert!(parse_index(&bad).is_empty());}
    }
    #[test]
    fn upload_identity_includes_layout_format_type_and_contents() {
        let a=key(256,256,0x1908,0x8033,b"test");
        for b in [key(128,256,0x1908,0x8033,b"test"),key(256,256,0x1907,0x8033,b"test"),key(256,256,0x1908,0x1401,b"test"),key(256,256,0x1908,0x8033,b"different")] {assert_ne!(a,b);}
    }
    #[test]
    fn replacement_png_preserves_color_under_partial_alpha() {
        let png=[137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,1,0,0,0,1,8,6,0,0,0,31,21,196,137,0,0,0,13,73,68,65,84,120,156,99,56,145,98,212,0,0,5,53,1,223,115,98,227,219,0,0,0,0,73,69,78,68,174,66,96,130];
        assert_eq!(Image::from_bytes_straight(&png).unwrap().pixels(),&[200,100,50,128]);
        assert_eq!(Image::from_bytes(&png).unwrap().pixels(),&[100,50,25,128]);
    }
}
