
# rust-modem

用 Rust 写的 USB 蜂窝模组（QMI / MBIM）协议栈。从 USB 抽象层、CDC WDM 传输，
到 QMI 与 MBIM 的编解码和客户端：协议部分（`rm-usb-core`、`rm-cdc-wdm`、
`rm-qmi`、`rm-mbim`、`rm-client-core`）都是 `no_std`，只依赖 `alloc`；
USB 传输层则抽成 trait，由后端实现，因此同一套协议代码既能跑在原生平台
（`rm-usb-nusb`，基于 [nusb](https://github.com/kevinmehall/nusb)），
也能跑在浏览器里（`rm-usb-webusb`，WebUSB）。

## 分层结构

```
rm-qmi / rm-mbim      QMI、MBIM 的编解码与客户端，都实现 rm-client-core 的 Uim
   └── rm-cdc-wdm     把协议消息塞进 CDC WDM 控制通道
          └── rm-usb-core   与传输无关的 USB 抽象（三个 trait）
                 ├── rm-usb-nusb     原生：Linux / Windows / macOS / Android（fd）
                 └── rm-usb-webusb   浏览器：wasm32 + WebUSB
```

原生后端在桌面上自己枚举并打开设备；Android 上半样都做不了——USB Host API
是唯一入口，它交回来的是一个已经打开的文件描述符，所以那边用
`NusbDevice::from_fd` 包住这个 fd，之后的一切完全相同。想跑 QMI / MBIM 的话，
由申请权限、`openDevice()` 的那一侧把描述符交进来即可（例如 Flutter 应用里
`UsbDeviceConnection.getFileDescriptor()` 的返回值）。

`rm-client-core` 提供与协议无关的客户端接口（目前是 `Uim`：卡状态、APDU 收发、
逻辑通道），`rm-qmi` 与 `rm-mbim` 都实现它，所以上层代码可以只针对一种协议来写。

`rm-qmi` 里的 `Transport` 是 QMI 客户端脚下的消息管道，它带着 service id，因为每个
QMI 消息都属于某个 service。USB 上所有 service 共用一条 CDC WDM 接口、service id
写在帧里；QRTR 上每个 service 是总线上的一个端口，得先向名字服务查它的位置。MBIM
没有 service 这一层，所以不参与这个抽象，仍旧直接对着 `CdcWdm` 写。

QRTR 本体是 `rm-qmi` 里的一个模块，由 `qrtr` feature 打开（默认关闭）：它只在
Linux / Android 上有实现，其他平台留一个同形状的"总线不存在"版本，好让上层和生成的
绑定不依赖生成它们的那台机器。

## Send 与非 Send 变体

带 `async fn` 的 trait 都由 `trait-variant` 生成两份：`LocalXxx` 是原 trait，
`Xxx` 是返回 `impl Future + Send` 的变体。实现者按平台挑一份实现即可，另一份由宏
自动补上：

| trait      | 非 Send              | Send            | 本仓库谁实现                                                                           |
| ---------- | -------------------- | --------------- | -------------------------------------------------------------------------------------- |
| 总线       | `LocalUsbBus`      | `UsbBus`      | `rm-usb-nusb` 实现 `UsbBus`，`rm-usb-webusb` 实现 `LocalUsbBus`                |
| 设备       | `LocalUsbDevice`   | `UsbDevice`   | 同上                                                                                   |
| 传输       | `LocalUsbTransfer` | `UsbTransfer` | 同上                                                                                   |
| 消息管道   | `LocalTransport`   | `Transport`   | 定义在`rm-qmi`，由 `CdcWdm` 和它的 `qrtr` 模块实现（只有 QMI 有 service 这一层） |
| 协议客户端 | `LocalUsbClient`   | `UsbClient`   | `rm-qmi` / `rm-mbim` 实现 `LocalUsbClient`                                       |
| UIM        | `LocalUim`         | `Uim`         | 同上                                                                                   |

`UsbBusExt::request` 是唯一没有变体的 async trait：`trait_variant` 把 `async fn`
改写成 `fn -> impl Future + Send`，但方法体是原样复制的，所以带默认实现的 async
方法没法生成 Send 版本。

QMI/MBIM 客户端实现的是 `Local*` 那一份，因为 Send 变体要求整条调用链的 future
都是 `Send`，而 `rm-cdc-wdm` 的泛型代码（`CdcWdm<D>`）是照着 `Local*` 写的，
宏不会把泛型方法体“升级”成 Send 版本。要让客户端也用 Send，需要把 `rm-cdc-wdm` /
`rm-qmi` / `rm-mbim` 整体改成 `D: UsbDevice` 参数化，代价是 WebUSB（`JsValue`
不是 `Send`）不能再跑 QMI/MBIM。

## crate 一览

| crate                           | 说明                                                                                                                                                                                         |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [rm-usb-core](rm-usb-core)       | 与传输无关的 USB 抽象：`DeviceFilter` 匹配、`SetupPacket`、`Transfer`，`UsbBus` / `UsbDevice` / `UsbTransfer` 三个 trait，以及客户端接口 `UsbClient` 和 `bus.request::<C>()` |
| [rm-usb-nusb](rm-usb-nusb)       | 原生后端（`std`），基于 [nusb](https://github.com/kevinmehall/nusb)                                                                                                                         |
| [rm-usb-webusb](rm-usb-webusb)   | 浏览器后端（`wasm32` + `std`），基于 `web-sys` 的 WebUSB，只在 `wasm32` 下有内容                                                                                                     |
| [rm-cdc-wdm](rm-cdc-wdm)         | CDC WDM 驱动，对应内核`drivers/usb/class/cdc-wdm.c`：按描述符定位 DMM 接口，用 `SEND_ENCAPSULATED_COMMAND` / `GET_ENCAPSULATED_RESPONSE` 收发                                          |
| [rm-qmi](rm-qmi)                 | QMI 编解码与客户端，各服务的消息/TLV 类型由 libqmi 数据库生成，并内置内核`qmi_wwan` 的设备表；`qrtr` feature 打开 QRTR 传输                                                              |
| [rm-mbim](rm-mbim)               | MBIM 编解码与客户端，类型由 libmbim 数据库生成                                                                                                                                               |
| [rm-client-core](rm-client-core) | 协议无关的客户端抽象（`Uim` 及其状态类型）                                                                                                                                                 |
| [xtask](xtask)                   | 构建任务：抓取 / 生成设备表与服务类型（`std`，不发布）                                                                                                                                     |

## 环境要求

- Rust 1.87 或更高（edition 2024，且 `rm-mbim` 用了 1.87 稳定的 `is_multiple_of`）。
- 跑示例需要真机；纯编解码的单元测试不需要。

## 构建与测试

```bash
cargo build --workspace
cargo test --workspace
cargo doc --workspace --no-deps --open
```

## 示例

```bash
cargo run -p rm-qmi --example qmi_uim_state     # QMI 查卡状态
cargo run -p rm-mbim --example mbim_uim_state   # MBIM 查卡状态
cargo run -p rm-qmi --example get_eid           # 走 UIM 逻辑通道读 eUICC 的 EID（ES10c）
cargo run -p rm-qmi --features qrtr --example qrtr_uim_state   # 不走 USB，走 QRTR 查卡状态
```

客户端自身带着"我认哪种设备"的过滤器：`bus.request::<C>()` 会用 `C::CLIENT_FILTER`
向 bus 要一个设备并把 `C` 开上去，等价于先 `request_device` 再 `C::open`。

```rust
use rm_usb_core::{UsbBus, UsbBusExt};

type Client = rm_qmi::QmiClient<<NusbBus as UsbBus>::Device>;

let mut bus = NusbBus::new();
let mut client = bus.request::<Client>().await?;
```

示例就是这么拿客户端的，跑之前要保证：

- 模组已经插好，且目标接口没有被系统驱动独占。Linux 下若被 `qmi_wwan` /
  `cdc_mbim` 占用，先解绑（见 [docs/other/tools/install-wwan-unbind.sh](docs/other/tools/install-wwan-unbind.sh)）；
- Windows 下该接口需要绑定 WinUSB，步骤见
  [docs/other/windows-qmi-win-usb.md](docs/other/windows-qmi-win-usb.md)。

## 代码生成

`rm-qmi` 的设备表和两个 crate 的各服务 `types` 模块都是生成的，不要手改：

```bash
git submodule update --init ref/libqmi ref/libmbim

cargo xtask qmi-table      # rm-qmi/src/qmi_device.rs  ← 内核 qmi_wwan.c
cargo xtask qmi-codegen    # rm-qmi/src/types/        ← ref/libqmi/data
cargo xtask mbim-codegen   # rm-mbim/src/types/       ← ref/libmbim/data
```

`cargo xtask` 是 `.cargo/config.toml` 里定义的别名，等价于 `cargo run -p xtask --`。

## WebAssembly / WebUSB

```bash
cargo build -p rm-usb-webusb --target wasm32-unknown-unknown
```

`web-sys` 的 WebUSB 绑定仍在 unstable 门后，需要 `--cfg=web_sys_unstable_apis`，
`.cargo/config.toml` 里已经为 `wasm32-unknown-unknown` 配好。

## 文档

| 文档                                                                            | 内容                                                                                  |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| [docs/other/](docs/other/)                                                       | Quectel EM05-CE 在 WSL / Windows 上的实测笔记：USB 构成、AT 通道、解锁、QMI/MBIM 踩坑 |
| [docs/other/operations-guide.md](docs/other/operations-guide.md)                 | 操作指南：AT 通道、切换构成、验证、脚本索引                                           |
| [rm-qmi/src/lib.rs](rm-qmi/src/lib.rs) / [rm-mbim/src/lib.rs](rm-mbim/src/lib.rs) | crate 级文档（`cargo doc` 里更完整）                                                |

## 许可

按 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 双许可发布，二者任选其一。
