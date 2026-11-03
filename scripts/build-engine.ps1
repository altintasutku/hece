# Builds hece and the CUDA engine (release) on Windows.
#
#   powershell -File scripts/build-engine.ps1
#
# Needs the CUDA Toolkit (13.x) and cmake (from mise.toml). CTranslate2 forces the static CRT on
# Windows, so the whole build uses +crt-static and onnxruntime is loaded at run time
# (`ort-dynamic`). The CUDA path uses forward slashes: CMake's FindCUDA misreads backslashes.
$ErrorActionPreference = 'Stop'
$cuda = Get-ChildItem 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA' -Directory |
    Sort-Object Name -Descending | Select-Object -First 1
if (-not $cuda) { throw 'CUDA Toolkit not found' }
$env:CUDA_PATH = $cuda.FullName -replace '\\', '/'
$env:CUDA_TOOLKIT_ROOT_DIR = $env:CUDA_PATH
$env:PATH = "$($cuda.FullName)\bin;$env:PATH"
# Turing (7.5, RTX 20) is the oldest arch CUDA 13 supports; 8.0 SASS also runs on Ampere (8.6, RTX 30),
# 8.9 is Ada (RTX 40). An explicit list keeps CTranslate2's default (with archs CUDA 13 dropped) out;
# 12.0 is Blackwell (RTX 50xx).
$env:CUDA_ARCH_LIST = '7.5;8.0;8.9;12.0'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
mise x -- cargo build --release --locked -p hece -p hece-engine --features hece-engine/cuda,hece-engine/ort-dynamic
exit $LASTEXITCODE
