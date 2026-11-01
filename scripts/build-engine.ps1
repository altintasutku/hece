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
# 8.9 keeps CTranslate2's own arch list explicit (its default includes archs CUDA 13 dropped);
# 12.0 is Blackwell (RTX 50xx).
$env:CUDA_ARCH_LIST = '8.9;12.0'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
mise x -- cargo build --release -p hece -p hece-engine --features hece-engine/cuda,hece-engine/ort-dynamic
exit $LASTEXITCODE
