# scripts/build-android.ps1 — compila libaudio2mp3.so para Android (Windows).
#
#   .\scripts\build-android.ps1 [-Ndk <ruta>] [-Api 24] [-NoPackage]
#
# Requisitos: rustup y el NDK de Android (ANDROID_NDK_HOME, ANDROID_NDK_ROOT o
# el más reciente en $ANDROID_HOME\ndk o %LOCALAPPDATA%\Android\Sdk\ndk).
# Salida:
#   dist/android/jniLibs/<abi>/libaudio2mp3.so
#   dist/audio2mp3-android-v<versión>.zip y dist/SHA256SUMS
# Comprueba que los segmentos LOAD están alineados a 16 KB (Google Play) y que
# se exportan exactamente las funciones JNI definidas en audio2mp3-jni.
# Equivalente a scripts/build-android.sh.

param(
    [string]$Ndk,
    [int]$Api = 24,
    [switch]$NoPackage
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$PageSize = 16384

function Find-Ndk {
    if ($Ndk) { return $Ndk }
    foreach ($var in 'ANDROID_NDK_HOME', 'ANDROID_NDK_ROOT') {
        $value = [Environment]::GetEnvironmentVariable($var)
        if ($value -and (Test-Path $value)) { return $value }
    }
    $sdks = @($env:ANDROID_HOME, $env:ANDROID_SDK_ROOT, "$env:LOCALAPPDATA\Android\Sdk") | Where-Object { $_ }
    foreach ($sdk in $sdks) {
        if (Test-Path "$sdk\ndk") {
            $latest = Get-ChildItem "$sdk\ndk" -Directory |
                Sort-Object { [version]($_.Name -replace '[^0-9.]', '') } |
                Select-Object -Last 1
            if ($latest) { return $latest.FullName }
        }
    }
    throw 'No se encontró el NDK. Usa -Ndk <ruta> o define ANDROID_NDK_HOME.'
}

function Invoke-Native([string]$exe, [string[]]$arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "Falló: $exe $($arguments -join ' ')" }
}

$ndkPath = Find-Ndk
$bin = Join-Path $ndkPath 'toolchains\llvm\prebuilt\windows-x86_64\bin'
if (-not (Test-Path $bin)) { throw "NDK sin toolchain LLVM para Windows: $bin" }
$readelf = Join-Path $bin 'llvm-readelf.exe'
$nm = Join-Path $bin 'llvm-nm.exe'

$targets = @(
    @{ Rust = 'aarch64-linux-android'; Abi = 'arm64-v8a'; Clang = "aarch64-linux-android$Api-clang.cmd" },
    @{ Rust = 'armv7-linux-androideabi'; Abi = 'armeabi-v7a'; Clang = "armv7a-linux-androideabi$Api-clang.cmd" },
    @{ Rust = 'x86_64-linux-android'; Abi = 'x86_64'; Clang = "x86_64-linux-android$Api-clang.cmd" }
)

$cargoToml = Get-Content (Join-Path $root 'Cargo.toml') -Raw
$version = [regex]::Match($cargoToml, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$jniSource = Get-Content (Join-Path $root 'crates\audio2mp3-jni\src\lib.rs') -Raw
$expected = [regex]::Matches($jniSource, 'extern "system" fn (Java_\w+)') | ForEach-Object { $_.Groups[1].Value } | Sort-Object

Write-Host "audio2mp3 v$version | NDK $(Split-Path -Leaf $ndkPath) | API $Api"

$dist = Join-Path $root 'dist'
$jniLibs = Join-Path $dist 'android\jniLibs'
if (Test-Path (Join-Path $dist 'android')) { Remove-Item -Recurse -Force (Join-Path $dist 'android') }

Push-Location $root
try {
    $installed = rustup target list --installed
    $results = @()
    foreach ($t in $targets) {
        if ($installed -notcontains $t.Rust) { Invoke-Native 'rustup' @('target', 'add', $t.Rust) }
        $linker = Join-Path $bin $t.Clang
        if (-not (Test-Path $linker)) { throw "No existe el linker $linker (¿API $Api soportada por este NDK?)" }
        $envName = 'CARGO_TARGET_' + ($t.Rust.ToUpper() -replace '-', '_')
        Set-Item "env:${envName}_LINKER" $linker
        Set-Item "env:${envName}_RUSTFLAGS" "-C link-arg=-Wl,-z,max-page-size=$PageSize"
        try {
            Write-Host "-> $($t.Abi) ($($t.Rust))"
            Invoke-Native 'cargo' @('build', '--release', '--quiet', '-p', 'audio2mp3-jni', '--target', $t.Rust)
        } finally {
            Remove-Item "env:${envName}_LINKER", "env:${envName}_RUSTFLAGS" -ErrorAction SilentlyContinue
        }

        $so = Join-Path $root "target\$($t.Rust)\release\libaudio2mp3.so"
        # Alineación de todos los segmentos LOAD.
        $aligns = & $readelf -lW $so | Where-Object { $_ -match '^\s*LOAD\s' } |
            ForEach-Object { [Convert]::ToInt64((($_.Trim() -split '\s+')[-1] -replace '^0x', ''), 16) }
        if (-not $aligns -or ($aligns | Where-Object { $_ -lt $PageSize })) {
            throw "$($t.Abi): segmentos LOAD sin alinear a 16 KB ($($aligns -join ', '))"
        }
        # Símbolos JNI exportados = funciones definidas en el puente.
        $exported = & $nm -D --defined-only $so | ForEach-Object { ($_ -split '\s+')[-1] } |
            Where-Object { $_ -like 'Java_*' } | Sort-Object
        if (Compare-Object $expected $exported) {
            throw "$($t.Abi): los símbolos JNI exportados no coinciden con audio2mp3-jni"
        }

        $dest = Join-Path $jniLibs $t.Abi
        New-Item -ItemType Directory -Force $dest | Out-Null
        Copy-Item $so $dest
        $results += [pscustomobject]@{
            ABI      = $t.Abi
            KB       = [math]::Round((Get-Item $so).Length / 1KB, 1)
            Alineado = '16 KB'
            JNI      = $exported.Count
        }
    }
    $results | Format-Table -AutoSize | Out-String | Write-Host

    if (-not $NoPackage) {
        $zip = Join-Path $dist "audio2mp3-android-v$version.zip"
        if (Test-Path $zip) { Remove-Item -Force $zip }
        # Compress-Archive (PowerShell 5.1) guarda rutas con '\'; se crean a mano con '/'.
        Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($t in $targets) {
                $file = Join-Path $jniLibs "$($t.Abi)\libaudio2mp3.so"
                [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $archive, $file, "jniLibs/$($t.Abi)/libaudio2mp3.so",
                    [IO.Compression.CompressionLevel]::Optimal) | Out-Null
            }
        } finally {
            $archive.Dispose()
        }
        $sums = Join-Path $dist 'SHA256SUMS'
        $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
        $line = "$hash  $(Split-Path -Leaf $zip)"
        $others = @()
        if (Test-Path $sums) { $others = Get-Content $sums | Where-Object { $_ -notmatch [regex]::Escape((Split-Path -Leaf $zip)) } }
        [IO.File]::WriteAllLines($sums, [string[]](@($others) + $line))
        Write-Host "Paquete: $zip"
        Write-Host "SHA256:  $hash"
    }
} finally {
    Pop-Location
}
