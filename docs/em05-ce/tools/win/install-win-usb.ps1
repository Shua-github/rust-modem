# 给模组的某个接口装临时 WinUSB 驱动（自签证书 + 签名 catalog + 强制绑定）。
#
#   Start-Process powershell -Verb RunAs -Wait -ArgumentList @(
#     '-NoProfile','-ExecutionPolicy','Bypass','-File','…\install-win-usb.ps1',
#     '-InfPath','…\quectel-win-usb.inf',
#     '-HardwareId','USB\VID_2C7C&PID_0127&MI_06')
#
# 日志：%TEMP%\codex-win-usb\install.log
# 还原：pnputil /delete-driver <包名> /uninstall /force + 删除自签证书 + pnputil /scan-devices

param(
    [Parameter(Mandatory = $true)][string] $InfPath,
    [Parameter(Mandatory = $true)][string] $HardwareId
)

$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:TEMP 'codex-win-usb'
New-Item -ItemType Directory -Path $dir -Force | Out-Null

$inf = Join-Path $dir (Split-Path $InfPath -Leaf)
Copy-Item -LiteralPath $InfPath -Destination $inf -Force

$catalogLine = Select-String -Path $inf -Pattern '^CatalogFile\s*=\s*(.+)$'
if (-not $catalogLine) {
    throw "no CatalogFile entry in $inf"
}
$cat = Join-Path $dir $catalogLine.Matches[0].Groups[1].Value.Trim()
$log = Join-Path $dir 'install.log'

function Write-Log([string] $message) {
    Add-Content -Path $log -Value $message -Encoding utf8
}

Set-Content -Path $log -Value "== run $(Get-Date -Format s)" -Encoding utf8

Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class NewDev {
    [DllImport("newdev.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool UpdateDriverForPlugAndPlayDevicesW(
        IntPtr hwndParent,
        string hardwareId,
        string fullInfPath,
        uint installFlags,
        out bool rebootRequired);
}
"@

$cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject 'CN=Temporary WinUSB binding' `
    -CertStoreLocation 'Cert:\CurrentUser\My' `
    -KeyUsage DigitalSignature `
    -KeyExportPolicy Exportable `
    -NotAfter (Get-Date).AddDays(30)
Write-Log "certificate thumbprint = $($cert.Thumbprint)"

$cerPath = Join-Path $dir 'temporary.cer'
Export-Certificate -Cert $cert -FilePath $cerPath -Force | Out-Null
Import-Certificate -FilePath $cerPath -CertStoreLocation 'Cert:\LocalMachine\Root' | Out-Null
Import-Certificate -FilePath $cerPath -CertStoreLocation 'Cert:\LocalMachine\TrustedPublisher' | Out-Null
Write-Log 'certificate installed into Root + TrustedPublisher'

Remove-Item $cat -ErrorAction SilentlyContinue
New-FileCatalog -Path $inf -CatalogFilePath $cat -CatalogVersion 2 | Out-Null
$signature = Set-AuthenticodeSignature -FilePath $cat -Certificate $cert -HashAlgorithm SHA256
Write-Log "catalog signature status = $($signature.Status)"

Write-Log "pnputil:`n$(& pnputil.exe /add-driver $inf /install 2>&1 | Out-String)"

$reboot = $false
$ok = [NewDev]::UpdateDriverForPlugAndPlayDevicesW([IntPtr]::Zero, $HardwareId, $inf, 1, [ref] $reboot)
$lastError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
Write-Log "update device = $ok (reboot=$reboot, win32=$lastError)"
