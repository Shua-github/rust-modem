# 操作指南（EM05-CE + WSL）

## 1. 打开 AT 通道

模组在不同构成下 AT 口的接口号/端点号会变，三种拿法：

| 场景 | 方法 |
| --- | --- |
| 模组在 Windows，且 AT 功能开启 | 用设备管理器里的 AT 串口（形如 `COMx`）：`win/tools.ps1 -Action at-com -Port COMx -Commands 'ATI'` |
| 模组在 Windows，没有 COM 口 | 给 AT 接口临时绑 WinUSB（`win/install-win-usb.ps1`），再用 `win/modem.py at 'ATI'` |
| 模组挂在 WSL | `python3 docs/other/tools/quectel.py at 'ATI'`（AT 口/端点自动探测） |

WSL 侧发 AT（MBIM 构成下 AT 口是接口 0、bulk `0x01/0x81`）：

```bash
wsl -d Arch -- bash -lc 'python3 docs/other/tools/quectel.py status'
```

## 2. 常用查询

```
ATI                     # 型号 / 固件版本
AT+QCFG="usbnet"        # 2=MBIM, 0=QMI/RmNet, 3=RNDIS
AT+QCFG="usbcfg"        # 各功能开关（倒数第二位是 ADB）
AT+QADBKEY?             # ADB 解锁码
```

## 3. 切换 USB 构成

```
AT+QCFG="usbnet",2                                          # 切回 MBIM（WSL 侧最稳）
AT+QCFG="usbcfg",0x2C7C,0x0127,1,1,1,1,1,1,0                # 全功能 + ADB
AT+CFUN=1,1                                                 # 重启生效
```

注意：

- 这两条命令在有些固件上会立即触发重新枚举，别紧接着发下一条（等设备回来再发）。
- 改 `usbcfg` 时前两项 `vid,pid` 和最后一位要按 `AT+QCFG="usbcfg"` 的返回值照抄，
  只改需要的位（倒数第二位 = ADB）。

## 4. 解锁 ADB

完整流程见 [quectel-adb-unlock.md](quectel-adb-unlock.md)，一条命令版：

```bash
python3 docs/other/tools/quectel.py unlock-adb              # 默认把 usbcfg 的 ADB 位置 1
python3 docs/other/tools/quectel.py set-usbcfg 1,1,1,1,1,1,1 # 或显式指定全部功能位
```

验证（模组挂在 Windows 时）：

```powershell
& "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" devices -l
& "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" shell uname -a
```

## 5. 挂到 WSL 用 rm-cli

```powershell
usbipd list                          # 找到模组的 BUSID
usbipd attach --wsl --busid <BUSID>  # 需要管理员
```

如果模组当前被 Windows 驱动占用（Quectel 的 GNSS/AT/DM/MBIM/ADB 驱动都在），要先
`usbipd bind --force --busid <BUSID>`；usbipd 会提示可能需要重启，实测**拔插一次模组**即可生效
（`usbipd list` 里父设备会变成 `USBIP Shared Device`）。用完
`usbipd unbind --busid <BUSID>` 把设备还给 Windows，COM 口和 ADB 随之恢复。

```bash
wsl -d Arch -- bash docs/other/tools/usb_info.py report
wsl -d Arch -- bash -lc 'cd <仓库目录> && ./target/debug/rm-cli mbim uim state'
```

如果 `dmesg` 出现 `can't set config #1, error -110`，见
[wsl-usbipd-troubleshooting.md](wsl-usbipd-troubleshooting.md)。

## 6. 脚本索引

全部工具都在 **`docs/other/tools/`**，用法见 [tools/README.md](tools/README.md)。

共 4 个 WSL 脚本 + 4 个 Windows 脚本，清单与用法集中在
[tools/README.md](tools/README.md)：

- `tools/usb_info.py`、`tools/quectel.py`、`tools/probe.py`、`tools/repro.sh`
- `tools/install-wwan-unbind.sh`（永久解绑内核驱动）、`tools/wsl-chromium.sh`
- `tools/win/modem.py`、`tools/win/install-win-usb.ps1`、`tools/win/quectel-win-usb.inf`、`tools/win/tools.ps1`
