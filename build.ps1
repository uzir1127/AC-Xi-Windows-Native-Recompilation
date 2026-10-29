param(
    [Parameter(Mandatory=$true)][string]$IpaPath,
    [string]$LlvmMingwRoot = $env:ACXI_LLVM_MINGW,
    [string]$Python = 'python',
    [string]$Cargo = 'cargo',
    [int]$Jobs = 6,
    [switch]$Offline,
    [switch]$ReuseGenerated,
    [switch]$RunTests
)
$ErrorActionPreference = 'Stop'
if ([Environment]::Is64BitOperatingSystem -ne $true) { throw 'Windows x64 is required.' }
if (!$LlvmMingwRoot) { throw 'Pass -LlvmMingwRoot or set ACXI_LLVM_MINGW to your LLVM-MinGW folder.' }
$compilerBin = Join-Path (Resolve-Path -LiteralPath $LlvmMingwRoot).Path 'bin'
foreach ($name in @('x86_64-w64-mingw32-clang.exe','x86_64-w64-mingw32-clang++.exe','llvm-ar.exe','x86_64-w64-mingw32-windres.exe')) {
    if (!(Test-Path -LiteralPath (Join-Path $compilerBin $name))) { throw "Missing compiler tool: $name" }
}
foreach ($name in @($Python,$Cargo,'rustc','cmake','ninja')) {
    if (!(Get-Command $name -ErrorAction SilentlyContinue)) { throw "Tool unavailable: $name. See docs/BUILDING.md." }
}
$ipa = (Resolve-Path -LiteralPath $IpaPath).Path
$env:PATH = "$compilerBin;$env:PATH"
$env:CC = Join-Path $compilerBin 'x86_64-w64-mingw32-clang.exe'
$env:CXX = Join-Path $compilerBin 'x86_64-w64-mingw32-clang++.exe'
$env:AR = Join-Path $compilerBin 'llvm-ar.exe'
$env:RC = Join-Path $compilerBin 'x86_64-w64-mingw32-windres.exe'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = $env:CC
# A GNU-host Rust toolchain needs its own libgcc-capable linker for host build
# scripts. LLVM-MinGW's gcc alias intentionally uses compiler-rt instead.
$rustDetails = & rustc -vV
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the Rust host toolchain.' }
if ($rustDetails -match '^host: x86_64-pc-windows-gnu$') {
    $rustSysroot = (& rustc --print sysroot).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the Rust sysroot.' }
    $hostLinker = Join-Path $rustSysroot 'lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained/x86_64-w64-mingw32-gcc.exe'
    if (!(Test-Path -LiteralPath $hostLinker)) { throw 'The GNU Rust host is missing its self-contained linker.' }
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $hostLinker
}
$env:CXXSTDLIB = 'c++'
$env:CMAKE_GENERATOR = 'Ninja'
$env:CMAKE_POLICY_VERSION_MINIMUM = '3.5'
$env:CARGO_BUILD_JOBS = "$Jobs"
$env:ACXI_AOT_GENERATED = Join-Path $PSScriptRoot 'local/generated'
$env:ACXI_AOT_CACHE = Join-Path $PSScriptRoot 'local/cc-cache'
$env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot 'local/target'
$prepareArgs = @((Join-Path $PSScriptRoot 'tools/prepare.py'),$ipa)
if ($ReuseGenerated) { $prepareArgs += '--reuse-generated' }
& $Python @prepareArgs
if ($LASTEXITCODE -ne 0) { throw 'Preparing private game input failed.' }
$cargoArgs = @('build','--release','--locked','--target','x86_64-pc-windows-gnullvm')
if ($Offline) { $cargoArgs += '--offline' }
Push-Location (Join-Path $PSScriptRoot 'runtime')
try {
    & $Cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Build failed: $LASTEXITCODE" }
    if ($RunTests) {
        $testArgs = @('test','--release','--lib','--locked','--target','x86_64-pc-windows-gnullvm')
        if ($Offline) { $testArgs += '--offline' }
        $testTemp = Join-Path $PSScriptRoot 'local/test-tmp'
        New-Item -ItemType Directory -Path $testTemp -Force | Out-Null
        if ((Get-Item -LiteralPath $testTemp).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Test temporary directory must not be a link.' }
        $previousTemp = $env:TEMP
        $previousTmp = $env:TMP
        try {
            $env:TEMP = $testTemp
            $env:TMP = $testTemp
            & $Cargo @testArgs
            if ($LASTEXITCODE -ne 0) { throw "Runtime tests failed: $LASTEXITCODE" }
        } finally {
            $env:TEMP = $previousTemp
            $env:TMP = $previousTmp
        }
    }
} finally { Pop-Location }
Write-Output 'Build complete. The executable and translated game code are private outputs under local/.'
