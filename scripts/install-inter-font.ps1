$ErrorActionPreference = "Stop"

$fontDirectory = Join-Path $env:LOCALAPPDATA "Microsoft\Windows\Fonts"
$fontPath = Join-Path $fontDirectory "Inter.ttc"
$registryPath = "HKCU:\Software\Microsoft\Windows NT\CurrentVersion\Fonts"
$registryName = "Inter (TrueType)"

try {
    $registered = Get-ItemProperty -Path $registryPath -Name $registryName -ErrorAction SilentlyContinue
    if ((Test-Path $fontPath) -and $registered) {
        return
    }

    Write-Host "Installation de la police Inter pour Archia Browser..." -ForegroundColor DarkCyan
    $temporaryRoot = Join-Path $env:TEMP ("archia-inter-" + [guid]::NewGuid().ToString("N"))
    $archivePath = Join-Path $temporaryRoot "Inter-4.1.zip"
    $extractPath = Join-Path $temporaryRoot "extract"
    New-Item -ItemType Directory -Path $temporaryRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $fontDirectory -Force | Out-Null

    try {
        Invoke-WebRequest -Uri "https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip" -OutFile $archivePath
        Expand-Archive -LiteralPath $archivePath -DestinationPath $extractPath -Force

        $fontFile = Get-ChildItem -LiteralPath $extractPath -Filter "Inter.ttc" -File -Recurse |
            Select-Object -First 1
        if (-not $fontFile) {
            throw "Le fichier Inter.ttc est absent de l'archive officielle."
        }

        Copy-Item -LiteralPath $fontFile.FullName -Destination $fontPath -Force
        $licenseFile = Get-ChildItem -LiteralPath $extractPath -Filter "LICENSE.txt" -File -Recurse |
            Select-Object -First 1
        if ($licenseFile) {
            Copy-Item -LiteralPath $licenseFile.FullName -Destination (Join-Path $fontDirectory "Inter-OFL-LICENSE.txt") -Force
        }

        New-Item -Path $registryPath -Force | Out-Null
        New-ItemProperty -Path $registryPath -Name $registryName -Value $fontPath -PropertyType String -Force | Out-Null

        if (-not ("ArchiaFontBroadcast" -as [type])) {
            Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class ArchiaFontBroadcast {
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)]
    public static extern int AddFontResourceEx(string fileName, uint flags, IntPtr reserved);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam, uint flags, uint timeout, out IntPtr result);
}
"@
        }
        [void][ArchiaFontBroadcast]::AddFontResourceEx($fontPath, 0, [IntPtr]::Zero)
        $result = [IntPtr]::Zero
        [void][ArchiaFontBroadcast]::SendMessageTimeout(
            [IntPtr]0xffff, 0x001D, [IntPtr]::Zero, [IntPtr]::Zero, 0x0002, 1000, [ref]$result
        )
        Write-Host "Police Inter installée." -ForegroundColor Green
    }
    finally {
        if (Test-Path $temporaryRoot) {
            Remove-Item -LiteralPath $temporaryRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
catch {
    Write-Warning "Impossible d'installer Inter automatiquement : $($_.Exception.Message)"
    Write-Warning "Archia Browser utilisera la police système de remplacement."
}
