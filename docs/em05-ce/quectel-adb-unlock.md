# 解锁 EM05-CE 的 ADB（含密钥算法）

参考：[RM500Q 等移远 5G 模组解锁 ADB](https://leux.net/doc/RM500Q%E7%AD%89%E7%A7%BB%E8%BF%9C5G%E6%A8%A1%E7%BB%84%E8%A7%A3%E9%94%81ADB.html)
（本文按 EM05-CE 实测重写；密钥算法用 `openssl` 实现，不依赖 Python 的 `crypt` 模块。）

## 原理

模组 ADB 默认锁定：**解锁之前，`usbcfg` 里的 ADB 位写不进去**（写命令返回 OK，但读回来仍是 0）。
解锁需要由模组自己给出的“码”推出密钥：

```
key = crypt_md5("SH_adb_quectel", salt = <AT+QADBKEY? 返回的码>)[12:27]
```

等价命令：

```bash
hash=$(openssl passwd -1 -salt <码> SH_adb_quectel)   # 形如 $1$<码>$<22位>
key=${hash:12:15}                                     # 取第 12~26 位（共 15 字符）
```

校验：文档样例码 `12345678` →
`openssl passwd -1 -salt 12345678 SH_adb_quectel` →
`$1$12345678$0jXKXQwSwMxYoegx0S.I.1` → 第 12~26 位 = `0jXKXQwSwMxYoeg`，与文档一致。

## 本机实测

| 项 | 值 |
| --- | --- |
| `AT+QADBKEY?` | `+QADBKEY: <模组返回的码>` |
| 计算出的密钥 | `<15 位密钥>` |
| `AT+QADBKEY="<密钥>"` | `OK` |
| `AT+QCFG="usbcfg",0x2C7C,0x0127,1,1,1,1,1,1,0` | `OK`（解锁前这一步的 ADB 位会被忽略） |
| 重启后 Windows | `MI_09 Android Composite ADB Interface` |
| `adb devices` | 列表里出现设备，状态 `device` |
| `adb shell uname -a` | `Linux mdm9607 3.18.44 #1 PREEMPT ... armv7l GNU/Linux` |

## 操作步骤

1. 打开 AT 通道（见 [operations-guide.md](operations-guide.md)）。
2. 取码：

   ```
   AT+QADBKEY?
   ```

3. 算密钥（WSL / Linux）：

   ```bash
   openssl passwd -1 -salt <码> SH_adb_quectel        # <码> 来自上一步
   # 取输出的第 12~26 位
   ```

4. 写入密钥：

   ```
   AT+QADBKEY="<密钥>"
   ```

5. 打开 ADB 位（其它位保持与读回来的值一致）：

   ```
   AT+QCFG="usbcfg",0x2C7C,0x0127,1,1,1,1,1,1,0
   AT+QCFG="usbcfg"        # 确认 adb 位已变成 1
   ```

6. 重启模组：

   ```
   AT+CFUN=1,1
   ```

7. 验证（模组挂在 Windows 时）：

   ```powershell
   & "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" devices -l
   & "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" shell uname -a
   ```

   注：`adb devices` 里这台的序列号显示为 `?`，不影响使用。

## 现成脚本

[`tools/quectel.py unlock-adb`](tools/quectel.py) 把上面 2~5 步全做了：

```bash
# 自动取码、算密钥、写密钥、打开 ADB 位并回读确认
python3 docs/other/tools/quectel.py unlock-adb
python3 docs/other/tools/quectel.py set-usbcfg 1,1,1,1,1,1,1   # 显式指定全部功能位
```

脚本会自动在所有厂商接口里找出能响应 `AT` 的那个（AT 口在不同构成下的接口号/端点号会变）。
