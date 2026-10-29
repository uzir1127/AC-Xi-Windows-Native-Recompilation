/* ACXi native execution bridge, 2026-10-27. SPDX-License-Identifier: MPL-2.0
 * Content-addressed native object cache. Unrecorded objects from earlier
 * OUT_DIRs are intentionally not imported: their compiler flags are unknown.
 */
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod digest {
    use std::ffi::c_void;
    type Handle = *mut c_void;
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptOpenAlgorithmProvider(handle: *mut Handle, name: *const u16, provider: *const u16, flags: u32) -> i32;
        fn BCryptCreateHash(algorithm: Handle, hash: *mut Handle, object: *mut u8, object_len: u32, secret: *mut u8, secret_len: u32, flags: u32) -> i32;
        fn BCryptHashData(hash: Handle, input: *mut u8, size: u32, flags: u32) -> i32;
        fn BCryptFinishHash(hash: Handle, output: *mut u8, size: u32, flags: u32) -> i32;
        fn BCryptDestroyHash(hash: Handle) -> i32;
    }
    // Opening CNG providers is expensive. Keep one provider for this short
    // build-script process; only hash states are allocated per file.
    fn algorithm() -> Handle {
        static PROVIDER: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *PROVIDER.get_or_init(|| {
            let mut algorithm=std::ptr::null_mut();
            let name:Vec<u16>="SHA256\0".encode_utf16().collect();
            assert_eq!(unsafe { BCryptOpenAlgorithmProvider(&mut algorithm,name.as_ptr(),std::ptr::null(),0) },0,"Cannot open SHA-256 provider");
            algorithm as usize
        }) as Handle
    }
    pub struct Sha256 { hash: Handle }
    impl Sha256 {
        pub fn new() -> Self {
            let algorithm=algorithm();
            let mut hash = std::ptr::null_mut();
            let status = unsafe { BCryptCreateHash(algorithm, &mut hash, std::ptr::null_mut(), 0, std::ptr::null_mut(), 0, 0) };
            assert_eq!(status,0,"Cannot create SHA-256 state: {status:#x}");
            Self { hash }
        }
        pub fn update(&mut self, bytes: &[u8]) {
            for block in bytes.chunks(1024 * 1024) {
                assert_eq!(unsafe { BCryptHashData(self.hash, block.as_ptr() as *mut u8, block.len() as u32, 0) }, 0, "Cannot update SHA-256 state");
            }
        }
        pub fn finish(self) -> [u8; 32] {
            let mut out = [0u8; 32];
            assert_eq!(unsafe { BCryptFinishHash(self.hash, out.as_mut_ptr(), 32, 0) }, 0, "Cannot finish SHA-256 state");
            out
        }
    }
    impl Drop for Sha256 {
        fn drop(&mut self) { unsafe { BCryptDestroyHash(self.hash); } }
    }
}

#[cfg(not(windows))]
mod digest {
    // Portable build-host fallback. The Windows host uses native CNG above.
    pub struct Sha256 { bytes: Vec<u8> }
    impl Sha256 {
        pub fn new() -> Self { Self { bytes: Vec::new() } }
        pub fn update(&mut self, bytes: &[u8]) { self.bytes.extend_from_slice(bytes); }
        pub fn finish(mut self) -> [u8; 32] {
            const K: [u32;64] = [0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2];
            let mut h: [u32;8] = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
            let bits = (self.bytes.len() as u64).wrapping_mul(8);
            self.bytes.push(0x80);
            while self.bytes.len() % 64 != 56 { self.bytes.push(0); }
            self.bytes.extend_from_slice(&bits.to_be_bytes());
            for block in self.bytes.chunks_exact(64) {
                let mut w=[0u32;64];
                for i in 0..16 { w[i]=u32::from_be_bytes(block[i*4..i*4+4].try_into().unwrap()); }
                for i in 16..64 { let a=w[i-15];let b=w[i-2];w[i]=w[i-16].wrapping_add(a.rotate_right(7)^a.rotate_right(18)^(a>>3)).wrapping_add(w[i-7]).wrapping_add(b.rotate_right(17)^b.rotate_right(19)^(b>>10)); }
                let [mut a,mut b,mut c,mut d,mut e,mut f,mut g,mut hh]=h;
                for i in 0..64 { let t1=hh.wrapping_add(e.rotate_right(6)^e.rotate_right(11)^e.rotate_right(25)).wrapping_add((e&f)^(!e&g)).wrapping_add(K[i]).wrapping_add(w[i]);let t2=(a.rotate_right(2)^a.rotate_right(13)^a.rotate_right(22)).wrapping_add((a&b)^(a&c)^(b&c));hh=g;g=f;f=e;e=d.wrapping_add(t1);d=c;c=b;b=a;a=t1.wrapping_add(t2); }
                for (dst,v) in h.iter_mut().zip([a,b,c,d,e,f,g,hh]) { *dst=dst.wrapping_add(v); }
            }
            let mut out=[0u8;32];for (chunk,v) in out.chunks_exact_mut(4).zip(h) { chunk.copy_from_slice(&v.to_be_bytes()); }out
        }
    }
}

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }
fn tagged(hash: &mut digest::Sha256, bytes: &[u8]) { hash.update(&(bytes.len() as u64).to_le_bytes()); hash.update(bytes); }
fn file_hash(path: &Path) -> String {
    let mut stream = fs::File::open(path).unwrap_or_else(|e| panic!("Cannot hash {}: {e}", path.display()));
    let mut hash = digest::Sha256::new();let mut buffer = vec![0u8; 1024 * 1024];
    loop { let n=stream.read(&mut buffer).unwrap();if n==0 { break; }hash.update(&buffer[..n]); }
    hex(&hash.finish())
}
fn canonical(path: &Path) -> PathBuf {
    let resolved=fs::canonicalize(path).unwrap_or_else(|e|panic!("Cannot resolve {}: {e}",path.display()));
    #[cfg(windows)]
    {
        let text=resolved.to_string_lossy();
        if let Some(rest)=text.strip_prefix("\\\\?\\UNC\\") { return PathBuf::from(format!("\\\\{rest}")); }
        if let Some(rest)=text.strip_prefix("\\\\?\\") { return PathBuf::from(rest); }
    }
    resolved
}
fn resolve(directory: &Path, line: &str) -> PathBuf { let p=Path::new(line);canonical(&if p.is_absolute(){p.to_owned()}else{directory.join(p)}) }
fn header_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|e|e.to_str()).map(str::to_ascii_lowercase).as_deref(),Some("h"|"hh"|"hpp"|"hxx"|"inc"|"inl"|"def"))
}
fn collect_headers(root: &Path, seen: &mut BTreeSet<PathBuf>, headers: &mut BTreeSet<PathBuf>) {
    if !root.is_dir() { return; }
    let directory=canonical(root);if !seen.insert(directory.clone()) { return; }
    let mut entries=fs::read_dir(&directory).unwrap().map(|e|e.unwrap().path()).collect::<Vec<_>>();entries.sort();
    for path in entries { if path.is_dir() { collect_headers(&path,seen,headers); }else if path.is_file()&&header_file(&path) { headers.insert(canonical(&path)); } }
}
fn cache_valid(object: &Path) -> bool {
    let hash_path=object.with_extension("sha256");
    let Ok(expected)=fs::read_to_string(hash_path) else { return false; };
    object.is_file() && fs::metadata(object).is_ok_and(|m|m.len()>0) && file_hash(object)==expected.trim()
}
fn install_object(source: &Path, target: &Path) {
    let temporary=target.with_extension(format!("o.tmp-{}",std::process::id()));
    fs::copy(source,&temporary).unwrap();
    let digest=file_hash(&temporary);
    if let Err(err)=fs::rename(&temporary,target) {
        if !cache_valid(target) { panic!("Cannot publish cache object {}: {err}",target.display()); }
        let _=fs::remove_file(&temporary);return;
    }
    let hash_path=target.with_extension("sha256");
    let hash_tmp=hash_path.with_extension(format!("tmp-{}",std::process::id()));
    fs::write(&hash_tmp,digest+"\n").unwrap();fs::rename(hash_tmp,hash_path).unwrap();
}

fn main() {
    let mut check=digest::Sha256::new();check.update(b"abc");
    assert_eq!(hex(&check.finish()),"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","SHA-256 self-check failed");
    println!("cargo:rerun-if-env-changed=ACXI_AOT_GENERATED");
    println!("cargo:rerun-if-env-changed=ACXI_AOT_CACHE");
    let package=Path::new(env!("CARGO_MANIFEST_DIR"));
    let generated=canonical(&std::env::var_os("ACXI_AOT_GENERATED").map(PathBuf::from).unwrap_or_else(||package.join("../../../../local/generated")));
    let manifest=generated.join("sources.txt");let header=generated.join("acxi_aot.h");
    println!("cargo:rerun-if-changed={}",manifest.display());
    assert!(header.is_file(),"Missing {}. Generate ACXi AOT sources first.",header.display());
    let source_list=fs::read_to_string(&manifest).unwrap();
    let sources=source_list.lines().map(str::trim).filter(|s|!s.is_empty()&&!s.starts_with('#')).map(|s|resolve(&generated,s)).collect::<Vec<_>>();
    assert!(!sources.is_empty(),"AOT source manifest is empty");
    let mut build=cc::Build::new();
    build.include(&generated).std("c11").opt_level(2).debug(false).warnings(false)
        .flag_if_supported("-fno-strict-aliasing").flag_if_supported("-ffp-contract=off").flag_if_supported("/fp:strict");
    let mut include_dirs=vec![generated.clone()];
    let includes_manifest=generated.join("include_dirs.txt");println!("cargo:rerun-if-changed={}",includes_manifest.display());
    if let Ok(includes)=fs::read_to_string(&includes_manifest) { for line in includes.lines().map(str::trim).filter(|s|!s.is_empty()&&!s.starts_with('#')) { let path=resolve(&generated,line);build.include(&path);include_dirs.push(path); } }
    let defines_manifest=generated.join("defines.txt");println!("cargo:rerun-if-changed={}",defines_manifest.display());
    if let Ok(defines)=fs::read_to_string(&defines_manifest) { for line in defines.lines().map(str::trim).filter(|s|!s.is_empty()&&!s.starts_with('#')) { if let Some((name,value))=line.split_once('=') { build.define(name,value); }else { build.define(line,None); } } }
    let target=std::env::var("TARGET").unwrap();let profile=std::env::var("PROFILE").unwrap();
    let compiler=build.get_compiler();
    let mut config=digest::Sha256::new();tagged(&mut config,b"ACXi-native-object-cache-v1");
    tagged(&mut config,target.as_bytes());tagged(&mut config,profile.as_bytes());tagged(&mut config,std::env::var("HOST").unwrap().as_bytes());
    tagged(&mut config,file_hash(&package.join("build.rs")).as_bytes());
    tagged(&mut config,compiler.path().to_string_lossy().as_bytes());
    for argument in compiler.args() { tagged(&mut config,argument.to_string_lossy().as_bytes()); }
    for (name,value) in compiler.get_envs() { tagged(&mut config,name.to_string_lossy().as_bytes());tagged(&mut config,value.to_string_lossy().as_bytes()); }
    for name in ["CPATH","C_INCLUDE_PATH","CPLUS_INCLUDE_PATH","INCLUDE","SDKROOT","MACOSX_DEPLOYMENT_TARGET","CL","_CL_","SOURCE_DATE_EPOCH"] {
        println!("cargo:rerun-if-env-changed={name}");tagged(&mut config,name.as_bytes());tagged(&mut config,std::env::var_os(name).unwrap_or_default().to_string_lossy().as_bytes());
    }
    let version=compiler.to_command().arg("--version").output().expect("Cannot query C compiler version");
    assert!(version.status.success(),"C compiler --version failed");tagged(&mut config,&version.stdout);tagged(&mut config,&version.stderr);
    let mut compiler_files=BTreeSet::new();compiler_files.insert(canonical(compiler.path()));
    if let Some(bin)=compiler.path().parent() { for entry in fs::read_dir(bin).unwrap() { let path=entry.unwrap().path();let name=path.file_name().unwrap().to_string_lossy().to_ascii_lowercase();if path.is_file()&&((name.starts_with("clang-")&&name.ends_with(".exe"))||name.starts_with("libclang-cpp")||name.starts_with("libllvm")||name.starts_with("libc++")||name.starts_with("libunwind")) { compiler_files.insert(canonical(&path)); } } }
    for path in compiler_files { println!("cargo:rerun-if-changed={}",path.display());tagged(&mut config,path.to_string_lossy().as_bytes());tagged(&mut config,file_hash(&path).as_bytes()); }
    // Discover standard include directories through this exact compiler/target.
    let search=compiler.to_command().args(["-E","-x","c","-v","-"]).output().expect("Cannot discover C include directories");
    assert!(search.status.success(),"Cannot discover compiler headers: {}",String::from_utf8_lossy(&search.stderr));
    let mut in_search=false;
    for line in String::from_utf8_lossy(&search.stderr).lines() {
        if line.contains("search starts here:") { in_search=true;continue; }
        if line.contains("End of search list.") { in_search=false; }
        if in_search { let path=PathBuf::from(line.trim().trim_end_matches(" (framework directory)").trim_matches('"'));if path.is_dir() { include_dirs.push(canonical(&path)); } }
    }
    for source in &sources { println!("cargo:rerun-if-changed={}",source.display());include_dirs.push(source.parent().unwrap().to_owned()); }
    let mut headers=BTreeSet::new();let mut seen=BTreeSet::new();
    for directory in &include_dirs { collect_headers(directory,&mut seen,&mut headers); }
    // Cargo must also notice new headers that shadow a previously resolved
    // include. Tracking existing files alone cannot detect that addition.
    for directory in &seen { println!("cargo:rerun-if-changed={}",directory.display()); }
    // Hash all searchable headers, including newly added shadowing headers.
    let mut header_hash=digest::Sha256::new();
    for path in &headers { println!("cargo:rerun-if-changed={}",path.display());tagged(&mut header_hash,path.to_string_lossy().as_bytes());tagged(&mut header_hash,file_hash(path).as_bytes()); }
    let header_key=header_hash.finish();let config_key=hex(&config.finish());
    let cache_base=std::env::var_os("ACXI_AOT_CACHE").map(PathBuf::from).unwrap_or_else(||package.join("../../../../local/cc-cache"));
    let cache=cache_base.join(&target).join(&profile).join(&config_key[..16]);fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("config.sha256"),&config_key).unwrap();
    let mut keys=BTreeMap::new();let mut missing=Vec::new();
    for source in &sources {
        let mut key=digest::Sha256::new();tagged(&mut key,config_key.as_bytes());tagged(&mut key,&header_key);tagged(&mut key,source.to_string_lossy().as_bytes());tagged(&mut key,file_hash(source).as_bytes());
        let object=cache.join(format!("{}.o",hex(&key.finish())));
        if !cache_valid(&object) { missing.push(source.clone()); }
        keys.insert(source.clone(),object);
    }
    println!("cargo:warning=ACXi AOT C cache: {} reused, {} to compile; {} header files checked",sources.len()-missing.len(),missing.len(),headers.len());
    if !missing.is_empty() {
        let staging=PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("aot-object-build");fs::create_dir_all(&staging).unwrap();
        let mut compile=build.clone();compile.out_dir(&staging).files(&missing);
        let objects=compile.compile_intermediates();assert_eq!(objects.len(),missing.len());
        for (source,object) in missing.iter().zip(objects) { install_object(&object,keys.get(source).unwrap()); }
    }
    // Build's file list is empty here; only the integrity-checked cache objects
    // are archived. A Rust-only rebuild therefore invokes no C compilation.
    for source in &sources { build.object(keys.get(source).unwrap()); }
    build.compile("acxi_aot");
}
