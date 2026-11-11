# Télécharge les modèles ONNX depuis OpenCV Zoo et vérifie leurs empreintes SHA-256.
# Usage : powershell -ExecutionPolicy Bypass -File scripts\fetch-models.ps1
$ErrorActionPreference = 'Stop'
$models = Join-Path $PSScriptRoot '..\models'
$base = 'https://github.com/opencv/opencv_zoo/raw/main/models'
$files = @(
    @{ Url = "$base/face_detection_yunet/face_detection_yunet_2026may.onnx"; Name = 'face_detection_yunet_2026may.onnx' },
    @{ Url = "$base/face_recognition_sface/face_recognition_sface_2021dec.onnx"; Name = 'face_recognition_sface_2021dec.onnx' },
    @{ Url = "$base/face_detection_yunet/LICENSE"; Name = 'LICENSE-yunet' },
    @{ Url = "$base/face_recognition_sface/LICENSE"; Name = 'LICENSE-sface' }
)
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
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $models $name)).Hash.ToLower()
    if ($actual -eq $expected) { Write-Host "OK      $name" }
    else { Write-Host "ÉCHEC   $name ($actual)"; $ok = $false }
}
if (-not $ok) { exit 1 }
