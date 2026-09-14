# 工具索引

和 [../README.md](../README.md) 里的文档配套。命令都假设当前目录是仓库根。

## WSL 侧

| 工具 | 用法 | 作用 |
| --- | --- | --- |
| `usb_info.py` | `python3 … report` / `endpoints` / `config [vid] [pid]` / `raw-config` / `unbind <vid>` | 列出设备、接口、已绑定的驱动与端点；dump 配置描述符；重试 `SET_CONFIGURATION`；解绑内核驱动 |
| `quectel.py` | `python3 … status` / `at <AT…>` / `set-usbnet <0-3>` / `set-usbcfg <bits\|adb>` / `unlock-adb` / `reboot` | 通过原始 USB 发 AT，切换 `usbnet`/`usbcfg`，解锁并打开 ADB |
| `probe.py` | `python3 … qmi [--set-config] [--dtr] [--request]` / `mbim [--open] [--notify] [--fetch] [--data-alt N]` / `find-qmi` | 分步复现 QMI / MBIM 的 USB 交互，看卡在哪一步 |
| `repro.sh` | `bash … <rm-cli 参数…>`（`--bin` 或 `RM_CLI_BIN` 指定二进制） | 跑一条 rm-cli 命令并 strace 记录 usbfs ioctl |
| `install-wwan-unbind.sh` | `sudo bash …` / `sudo bash … uninstall` | 安装/卸载 `modprobe.d` + `udev` 配置，永久解绑 `qmi_wwan`/`cdc_mbim` |
| `wsl-chromium.sh` | `bash … [url]` | 在 WSLg 里启动 Linux Chromium（跑 `output/cli-web` 的 WebUSB 壳子） |

依赖：`python-pyusb`、`libusb`、`strace`（`pacman -S python-pyusb libusb strace`）。
默认设备 id 是 `2c7c:0127`，可用 `VID` / `PID` 环境变量覆盖（`probe.py` 还支持 `IFACE`）。

## Windows 侧

| 工具 | 用法 | 作用 |
| --- | --- | --- |
| `win/modem.py` | `python win/modem.py status` / `at <AT…>` / `dump` / `mbim-raw` | 在已绑 WinUSB 的接口上发 AT、dump 描述符、抓原始 MBIM 响应 |
| `win/install-win-usb.ps1` | `-InfPath <inf> -HardwareId <hwid>`，管理员运行 | 自签证书 + 签名 catalog，给指定接口装临时 WinUSB；日志 `%TEMP%\codex-win-usb\install.log` |
| `win/quectel-win-usb.inf` | 配合上面的脚本 | 覆盖 AT(GNSS)/MBIM/QMI 三个接口（`MI_00`、`MI_01`、`MI_06`）的 WinUSB INF |
| `win/tools.ps1` | `-Action at-com -Port COMx -Commands 'ATI'` / `-Action reenumerate -InstanceId '<实例号>'` | 用串口发 AT；软重枚举设备实例 |

Windows 侧需要 `pyusb` + `libusb-package`：

```powershell
python -m venv .venv-at
.\.venv-at\Scripts\python.exe -m pip install pyusb libusb-package
```

## 还原临时改动

```powershell
pnputil /enum-drivers                     # 找到临时 WinUSB 包的发布名
pnputil /delete-driver <包名> /uninstall /force
# 再删除 CN=Temporary WinUSB binding 的自签证书
# （CurrentUser\My、LocalMachine\Root、LocalMachine\TrustedPublisher 三处）
pnputil /scan-devices
```
