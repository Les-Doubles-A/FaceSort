# Construit la version portable (dossier + zip) et l'installeur de FaceSort.
# Usage : powershell -ExecutionPolicy Bypass -File scripts\package.ps1 [-SkipBuild]
# Prérequis : modèles téléchargés (scripts\fetch-models.ps1 -AuraFace),
#             Visual Studio Build Tools (runtime Visual C++), Inno Setup 6.
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "FaceSort $version"

if (-not $SkipBuild) {
    cargo build --release --bins
    if ($LASTEXITCODE -ne 0) { throw "échec de cargo build" }
}

$dist = Join-Path $root "dist"
$name = "FaceSort-$version-portable"
$out = Join-Path $dist $name
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force (Join-Path $out 'models') | Out-Null

# Exécutables et DirectML (fourni par la crate ort)
Copy-Item target\release\FaceSort.exe, target\release\facesort-cli.exe, target\release\DirectML.dll $out

# Runtime Visual C++ en déploiement local
$redist = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT" -Directory |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $redist) { throw "runtime Visual C++ introuvable (Visual Studio Build Tools)" }
foreach ($dll in 'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') {
    Copy-Item (Join-Path $redist.FullName $dll) $out
}

# Modèles, licences, empreintes
$models = 'face_detection_yunet_2026may.onnx', 'face_recognition_sface_2021dec.onnx', 'auraface_glintr100.onnx',
          'LICENSE-yunet', 'LICENSE-sface', 'LICENSE-auraface.md', 'SHA256SUMS'
foreach ($m in $models) {
    $src = Join-Path 'models' $m
    if (-not (Test-Path $src)) { throw "modèle manquant : $src (lancer scripts\fetch-models.ps1 -AuraFace)" }
    Copy-Item $src (Join-Path $out 'models')
}
Copy-Item README.md, THIRD_PARTY_NOTICES.md $out

# Contrôle : la CLI démarre depuis le dossier portable et trouve ses modèles
$env:FACESORT_MODELS = ''
& (Join-Path $out 'facesort-cli.exe') --db (Join-Path $env:TEMP "facesort-package-check.db") status | Out-Null
if ($LASTEXITCODE -ne 0) { throw "la CLI du dossier portable ne démarre pas" }
Remove-Item -ErrorAction SilentlyContinue (Join-Path $env:TEMP "facesort-package-check.db*")

# Zip portable
$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $out -DestinationPath $zip -CompressionLevel Optimal
Write-Host "Portable : $zip"

# Installeur
$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 introuvable" }
& $iscc /Q "/DMyAppVersion=$version" "/DSourceDir=$out" installer\FaceSort.iss
if ($LASTEXITCODE -ne 0) { throw "échec d'Inno Setup" }
Write-Host "Installeur : $(Join-Path $root "installer\Output\FaceSort-Setup-$version.exe")"
