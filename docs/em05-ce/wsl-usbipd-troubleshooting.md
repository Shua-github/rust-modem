# usbipd 转发失败：can't set config #1, error -110

## 现象

模组 attach 到 WSL 后，内核日志里出现：

```
usb 1-1: config 1 has an invalid interface number: 9 but max is 5
usb 1-1: config 1 has no interface number 4
usb 1-1: can't set config #1, error -110
vhci_hcd: connection closed -> disconnect device
```

表现为设备一直没有接口（`/sys/bus/usb/devices/1-1:1.*` 不存在），几秒后 vhci 断链、
设备回到 Windows，看起来像“设备重枚举”。

## 根因

不是模组坏，也不是 rm-cli 的问题，而是 **usbipd-win 5.1.0 提交 URB 失败**。
证据在 Windows 应用程序事件日志（来源 `usbipd`）：

```
An exception occurred while communicating with the client:
System.ComponentModel.Win32Exception (87): DeviceIoControl returned error ERROR_INVALID_PARAMETER
   at Usbipd.AttachedEndpoint.<HandleSubmitAsync>d__11.MoveNext()
   at Usbipd.AttachedClient.<RunAsync>d__12.MoveNext()
```

查日志：

```powershell
Get-WinEvent -FilterHashtable @{ LogName='Application'; StartTime=(Get-Date).AddMinutes(-30) } |
    Where-Object { $_.Message -match 'Usbipd' } |
    Select-Object -First 5 TimeCreated, LevelDisplayName, Message | Format-List
```

## 触发条件：接口号不连续

这颗模组按功能号分配接口号，同一配置里会缺号。实测对照：

| 构成 | 接口号 | usbipd 转发 |
| --- | --- | --- |
| MBIM（`usbnet=2`，出厂 usbcfg） | 0,1,2 连续 | 通过 |
| 全功能（无 ADB） | 0,1,2,3,4,5 连续 | 通过 |
| 全功能（含 ADB） | 0,1,2,3,5,9（`bNumInterfaces`=7） | 失败 `ERROR_INVALID_PARAMETER` |
| QMI（`usbnet=0`） | 0,6 | 失败 |
| RNDIS（`usbnet=3`） | 0,9,10 | 失败 |

即：只要 USB 描述符里的接口号是稀疏的，usbipd 5.1.0 就转发不了（Windows 自己的 USB 栈没这个问题）。

**升级到 5.3.0 后实测仍然是同样的 `Win32Exception (87) 参数错误`**，即该问题未在新版本修复
（2026-09 实测：usbipd-win 5.3.0 + 驱动 7.2.2）。因此含 ADB 的构成只能在 Windows 侧使用。

## 排查手段（可复用）

```bash
# 看模组接口与驱动绑定
wsl -d Arch -- bash docs/other/tools/usb_info.py report

# 看接口 + 端点（接口号稀疏一眼就能看出来）
wsl -d Arch -- bash docs/other/tools/usb_info.py endpoints

# 看内核日志
wsl -d Arch -- bash -lc 'dmesg | tail -20'
```

Windows 侧：

```powershell
usbipd list
Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like '*VID_2C7C*' }
```

## 处理建议

1. 升级 usbipd：`winget upgrade dorssel.usbipd-win`（5.1.0 → 5.3.0，驱动 7.2.2），
   升级后重试上面的构成，看 `ERROR_INVALID_PARAMETER` 是否消失。
2. 需要用 ADB 时，把模组留在 Windows 上（全功能构造成立，`adb devices` 可用），
   或用一台真正的 Linux 主机直连（不经 usbip，稀疏接口号在物理 Linux 上没问题）。
3. 必须走 WSL 时，退回接口连续的构成，例如 `usbnet=2` + 出厂 usbcfg（MBIM）。

## 升级 usbipd 后的注意事项（实测）

升级到 5.3.0 之后，如果设备当前被 Windows 驱动占用，`attach` 会报：

```
WSL usbip: error: Attach Request for 3-1 failed - Device busy (exported)
usbipd: warning: The device appears to be used by Windows; stop the software using the device,
                 or bind the device using the '--force' option.
```

按提示执行 `usbipd bind --force --busid 3-1` 后，usbipd 会提示
`A reboot may be required before the changes take effect.`；此时 `pnputil /restart-device`
也会拒绝执行（“设备正在等待系统重新启动”）。**需要重启 Windows（或至少拔插一次设备）
让驱动切换生效**，之后 `attach` 才会成功。

顺带一提：`usbipd list` 里的状态会显示为 `Shared (forced)`，设备树里父设备会变成
`USBIP Shared Device`，但子接口的驱动要等重启/重插才会真正摘掉。
