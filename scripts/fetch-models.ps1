# Télécharge les modèles ONNX et vérifie leurs empreintes SHA-256.
# Usage : powershell -ExecutionPolicy Bypass -File scripts\fetch-models.ps1 [-AuraFace]
#   -AuraFace : ajoute le modèle de reconnaissance AuraFace-v1 (Apache 2.0, 261 Mo),
#               à utiliser avec la variable FACESORT_EMBEDDER=auraface.
param([switch]$AuraFace)
$ErrorActionPreference = 'Stop'
$models = Join-Path $PSScriptRoot '..\models'
$zoo = 'https://github.com/opencv/opencv_zoo/raw/main/models'
$files = @(
    @{ Url = "$zoo/face_detection_yunet/face_detection_yunet_2026may.onnx"; Name = 'face_detection_yunet_2026may.onnx' },
    @{ Url = "$zoo/face_recognition_sface/face_recognition_sface_2021dec.onnx"; Name = 'face_recognition_sface_2021dec.onnx' },
    @{ Url = "$zoo/face_detection_yunet/LICENSE"; Name = 'LICENSE-yunet' },
    @{ Url = "$zoo/face_recognition_sface/LICENSE"; Name = 'LICENSE-sface' }
)
if ($AuraFace) {
    $files += @{ Url = 'https://huggingface.co/fal/AuraFace-v1/resolve/main/glintr100.onnx'; Name = 'auraface_glintr100.onnx' }
    $files += @{ Url = 'https://huggingface.co/fal/AuraFace-v1/resolve/main/LICENSE.md'; Name = 'LICENSE-auraface.md' }
}
New-Item -ItemType Directory -Force $models | Out-Null
foreach ($f in $files) {
    $dest = Join-Path $models $f.Name
    if (Test-Path $dest) { Write-Host "Déjà présent : $($f.Name)"; continue }
    Write-Host "Téléchargement : $($f.Name)"
    Invoke-WebRequest -Uri $f.Url -OutFile $dest -UseBasicParsing
}
$ok = $true
foreach ($line in Get-Content (Join-Path $models 'SHA256SUMS')) {
    $parts = $line -split '\s+', 2
    $expected = $parts[0].ToLower()
    $name = $parts[1].TrimStart('*')
    $path = Join-Path $models $name
    if (-not (Test-Path $path)) { Write-Host "absent  $name (facultatif)"; continue }
    $actual = (Get-FileHash -Algorithm SHA256 $path).Hash.ToLower()
    if ($actual -eq $expected) { Write-Host "OK      $name" }
    else { Write-Host "ÉCHEC   $name ($actual)"; $ok = $false }
}
if (-not $ok) { exit 1 }
