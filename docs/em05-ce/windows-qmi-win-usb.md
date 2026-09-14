# Windows 下用 nusb（WinUSB）跑 QMI

**实测结论：可用**（2026-09，Windows 11 + rm-cli 的 nusb 后端）。

```
> target\debug\rm-cli.exe qmi devices
{ "devices": [ { "interfaces": [ ... ] } ] }

> target\debug\rm-cli.exe qmi uim state
{ "protocol": "qmi", "state": { "1": { "atr": [...], "imei": "<imei>", "ready": true } } }
```

## 前提

1. **模组在 QMI 构成**：`AT+QCFG="usbnet",0`
   （切构成后 AT 口会以 `COMx` 出现，可用 `docs/other/tools/win/tools.ps1 -Action at-com` 发；
   该命令会立即触发重新枚举，等设备回来再继续）。
   Windows 上这时会出现 `MI_06`（class `ff:ff:ff`，无驱动）= QMI 接口。
2. **rm-qmi 的厂商表里有 `2c7c:0127`**：上游 `qmi_wwan.c` 没有这条，
   本仓库在 `xtask/src/qmi_wwan.rs` 的 `LOCAL_ENTRIES` 里补了
   `QmiEntry::class(0x2c7c, Some(0x0127), 0xff, 0xff, 0xff, true)`，
   `cargo xtask qmi-table` 重新生成时会保留。
3. **编译 Windows 版 CLI**：`cargo build -p rm-cli`（nusb 走 WinUSB）。

## 步骤

```powershell
# 1. 给 QMI 接口（MI_06）绑 WinUSB；脚本自建证书 + 签名 catalog + 强制绑定
#    日志：%TEMP%\codex-win-usb\install.log
Start-Process powershell -Verb RunAs -Wait -ArgumentList @(
  '-NoProfile','-ExecutionPolicy','Bypass',
  '-File','docs\other\tools\win\install-win-usb.ps1',
  '-InfPath','docs\other\tools\win\quectel-win-usb.inf',
  '-HardwareId','USB\VID_2C7C&PID_0127&MI_06'
)

# 2. 确认驱动是 WINUSB
Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like '*VID_2C7C&PID_0127&MI_06*' } |
    ForEach-Object { Get-PnpDeviceProperty -InstanceId $_.InstanceId |
        Where-Object KeyName -match 'DriverDesc|Service' } | Format-Table -AutoSize

# 3. 跑 QMI
target\debug\rm-cli.exe qmi devices
target\debug\rm-cli.exe qmi uim state
```

## 为什么 Windows 行、WSL 不行

- Windows 侧 nusb 直接通过 **WinUSB** 对接口做控制/批量传输，不经过 usbipd 的 URB 转发；
  WinUSB 还会拒绝对已配置设备重发 `SET_CONFIGURATION`，没有副作用。
- WSL 侧要先经 usbipd 转发，而 QMI 构成的接口号是稀疏的（`0,6`），
  usbipd 会以 `ERROR_INVALID_PARAMETER` 拒掉 URB（见
  [wsl-usbipd-troubleshooting.md](wsl-usbipd-troubleshooting.md)）。

## 还原

```powershell
# 驱动包名以 pnputil /enum-drivers 的输出为准（本次是 oem45.inf）
pnputil /delete-driver oem45.inf /uninstall /force
# 删除自签名证书（CN=Codex Temporary USB Driver，位于 CurrentUser\My / LocalMachine\Root /
# LocalMachine\TrustedPublisher 三处）
pnputil /scan-devices
```
