# Quectel EM05-CE 使用笔记

本目录记录这块 **Quectel EM05-CE**（`2c7c:0127`，固件 `EM05CEFCR08A22M1G_LNV`）在
WSL / Windows 上的实际使用方式，以及排查过程中的结论和踩坑。

| 文档 | 内容 |
| --- | --- |
| [quectel-em05-usb-compositions.md](quectel-em05-usb-compositions.md) | 模组的 USB 构成、`usbnet` / `usbcfg` 含义、各构成实测结果 |
| [quectel-adb-unlock.md](quectel-adb-unlock.md) | 解锁 ADB 的完整流程（含密钥算法与验证方法） |
| [wsl-usbipd-troubleshooting.md](wsl-usbipd-troubleshooting.md) | usbipd 转发失败（`can't set config #1, error -110`）的原因与排查手段 |
| [windows-qmi-win-usb.md](windows-qmi-win-usb.md) | Windows 下用 nusb(WinUSB) 跑 QMI 的完整步骤（实测可用） |
| [mbim-uicc-ref-byte-array.md](mbim-uicc-ref-byte-array.md) | MBIM 的 UICC 参考字节数组是 `[length][offset]`：ATR 被截断成 8 字节的根因与修复 |
| [operations-guide.md](operations-guide.md) | 操作指南：AT 通道、切换构成、解锁、验证、脚本索引 |
| [tools/](tools/) | 本次用到的全部脚本（WSL 侧诊断/AT/解锁 + Windows 侧 AT），见 [tools/README.md](tools/README.md) |

所有命令都在下面的环境里实测过：

- Windows 11 + usbipd-win 5.1.0（可升 5.3.0）
- WSL2 Arch（`systemd=true`，内核 `6.18.33.2-microsoft-standard-WSL2`）
- 模组通过 usbipd 以 `3-1` 总线号挂在 WSL 上
