# EM05-CE 的 USB 构成

## 基本事实

| 项 | 值 |
| --- | --- |
| VID:PID | `2c7c:0127` |
| 型号 | Quectel EM05-CE |
| 固件 | `EM05CEFCR08A22M1G_LNV` |
| 平台 | `Linux mdm9607 3.18.44 armv7l`（ADB shell 里看到的） |

## 两个旋钮：`usbnet` 与 `usbcfg`

```
AT+QCFG="usbnet"        # 数据通道模式
AT+QCFG="usbcfg"        # 每个 USB 功能是否启用
```

`usbnet` 实测取值：

| 值 | 构成 | 实测结果 |
| --- | --- | --- |
| `2` | MBIM | 正常，Windows 出现 `MI_01 Net`，WSL 侧 `rm-cli mbim uim state` 可用 |
| `0` | QMI / RmNet | 模组侧正常（厂商接口 `ff:ff:ff` + AT），但见 usbipd 问题 |
| `3` | RNDIS | 模组侧可用，Windows RNDIS 驱动 Code 10，Linux 侧配不上 |
| `1` | ECM | 未实测 |

`usbcfg` 返回形如（前两项是 vid/pid，后面 7 位是功能开关）：

```
+QCFG: "usbcfg",0x2C7C,0x0127,b0,b1,b2,b3,b4,b5,b6
```

本机实测到的两位（其余未逐位验证）：

| 位 | 含义 | 验证方式 |
| --- | --- | --- |
| `b5`（倒数第二） | **ADB** | 置 1 后出现 `MI_09 Android Composite ADB Interface`（需先解锁，否则该位被忽略） |
| `b6`（最后一位） | **NMEA** | 置 1 后新增 `MI_04 Quectel USB NMEA Port` |

出厂值：`0,0,1,0,1,0,0`（只开 AT 与 rmnet）。

全功能值（本次设置，ADB + NMEA 全开）：

```
AT+QCFG="usbcfg",0x2C7C,0x0127,1,1,1,1,1,1,1
```

该值（配合 `usbnet=2`）在 Windows 上的接口：

| MI | 名称 | 说明 |
| --- | --- | --- |
| `MI_00` | Quectel GNSS Sensor Device | GNSS |
| `MI_01` | Quectel EM05-CE (Net) | MBIM 控制 |
| `MI_02` | —（无 Windows 驱动） | MBIM 数据 |
| `MI_03` | Quectel USB DM Port | 诊断口 |
| `MI_04` | Quectel USB NMEA Port | NMEA（最后一位开出来的） |
| `MI_05` | Quectel USB AT Port | AT 口 |
| `MI_09` | Android Composite ADB Interface | ADB |

## 接口编号是不连续的（重点）

这颗模组按“功能号”分配接口号，于是同一个配置里会出现空号；实测（Windows 的 MI 号 /
Linux 内核的告警）：

| 构成 | 接口号（实测） | `bNumInterfaces` | usbip 转发 |
| --- | --- | --- | --- |
| MBIM（`usbnet=2` + 出厂 usbcfg `0,0,1,0,1,0,0`） | 0,1,2 | 3 | ✅ 正常 |
| 全功能（无 ADB）`1,1,1,1,1,0,0` | 0,1,2,3,4,5 | 6 | ✅ 正常 |
| 全功能 + ADB `1,1,1,1,1,1,0` | 0,1,2,3,5,9 | 7 | ❌ 失败 |
| 全功能 + ADB + NMEA `1,1,1,1,1,1,1` | 0,1,2,3,4,5,9 | —（9 越界） | ❌ 失败 |
| QMI（`usbnet=0`） | 0,6 | 2 | ❌ 失败 |
| RNDIS（`usbnet=3`） | 0,9,10 | 3 | ❌ 失败 |

**ADB 天生稀疏**：`adb` 这个功能固定占接口号 9，而 `bNumInterfaces` 只统计“启用的功能个数”，
于是只要开 ADB，描述符就不自洽（9 > 上限）。把最后一位 NMEA 也打开只是把 4 号补上，
9 号依然越界。结论：**ADB 无法经 usbipd 转发**，
要用 ADB 就把模组留在 Windows（或在物理 Linux 主机上直连）。

Windows 容忍这种描述符，Linux 内核只会打印警告并继续，但 usbipd-win 5.1.0 会在转发时失败，
详见 [wsl-usbipd-troubleshooting.md](wsl-usbipd-troubleshooting.md)。

## 与 rm-cli 的关系

- MBIM：`rm-cli mbim uim state` 直接可用（代码里已修好配对数据接口 altsetting 的激活）。
- QMI：模组切到 `usbnet=0`，并在 `rm-qmi` 的厂商表里有 `2c7c:0127`
  （已在 `xtask/src/qmi_wwan.rs` 的 `LOCAL_ENTRIES` 补上，重新生成不会丢）。
  **Windows 下可用**（WinUSB 绑 `MI_06`，见 [windows-qmi-win-usb.md](windows-qmi-win-usb.md)）；
  经 usbipd 到 WSL 仍受稀疏接口号问题阻塞。
