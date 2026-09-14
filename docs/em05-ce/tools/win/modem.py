#!/usr/bin/env python3
"""Windows-side helper for a Quectel modem bound to WinUSB.

Subcommands:
  status        在 AT 接口上跑 ATI / usbnet / usbcfg / QADBKEY
  at <命令...>   在 AT 接口上发任意 AT 命令
  dump          打印设备的配置描述符（接口与端点）
  mbim-raw      发一条 MBIM 命令并打印原始响应（默认查 UICC ATR）

需要：pyusb + libusb-package（python -m pip install pyusb libusb-package），
并且目标接口已用 win/install-win-usb.ps1 绑好 WinUSB。
AT 接口、MBIM 控制接口、QMI 接口分别是 MI_00 / MI_01 / MI_06。

`VID`/`PID` 环境变量可覆盖默认的 2c7c:0127。
"""

import argparse
import os
import struct
import sys
import time

import libusb_package
import usb.core
import usb.util

VENDOR = int(os.environ.get("VID", "2c7c"), 16)
PRODUCT = int(os.environ.get("PID", "0127"), 16)

# MS UICC Low Level Access（rm-mbim/src/types/ms_uicc_low_level_access.rs）
UICC_LLA = bytes([0xC2, 0xF6, 0x58, 0x8E, 0xF0, 0x37, 0x4B, 0xC9,
                  0x86, 0x65, 0xF4, 0xD4, 0x4B, 0xD0, 0x93, 0x67])
ATR_CID = 1


def log(message):
    print(message, flush=True)


def find_device():
    backend = libusb_package.get_libusb1_backend()
    device = usb.core.find(idVendor=VENDOR, idProduct=PRODUCT, backend=backend)
    if device is None:
        raise SystemExit("device not found")
    log(f"device {device.idVendor:04x}:{device.idProduct:04x}")
    return device


def bulk_endpoints(interface):
    out = inp = 0
    for endpoint in interface:
        address = endpoint.bEndpointAddress
        if endpoint.bmAttributes & 0x03 != usb.util.ENDPOINT_TYPE_BULK:
            continue
        if address & 0x80:
            inp = address
        else:
            out = address
    return out, inp


def at_port(device, number):
    interface = usb.util.find_descriptor(
        device.get_active_configuration(), bInterfaceNumber=number)
    if interface is None:
        raise SystemExit(f"interface {number} not found")
    out, inp = bulk_endpoints(interface)
    if not out or not inp:
        raise SystemExit(f"interface {number} has no bulk endpoints")
    usb.util.claim_interface(device, number)
    log(f"AT port: interface {number}, out 0x{out:02x}, in 0x{inp:02x}")
    return out, inp


def read_at(device, endpoint_in, seconds=2.0):
    deadline = time.monotonic() + seconds
    chunks = []
    while time.monotonic() < deadline:
        try:
            data = device.read(endpoint_in, 512, timeout=400)
        except usb.core.USBTimeoutError:
            if chunks:
                break
            continue
        except usb.core.USBError as error:
            return f"<read error {error}>"
        chunks.append(bytes(data))
        if b"OK" in data or b"ERROR" in data:
            break
    return b"".join(chunks).decode("ascii", "replace").strip()


def command(device, endpoint_out, endpoint_in, text):
    device.write(endpoint_out, (text + "\r\n").encode(), timeout=2000)
    answer = read_at(device, endpoint_in)
    log(f"> {text}\n{answer}")
    return answer


def cmd_status(device, args):
    endpoint_out, endpoint_in = at_port(device, args.interface)
    for text in ("ATI", 'AT+QCFG="usbnet"', 'AT+QCFG="usbcfg"', "AT+QADBKEY?"):
        command(device, endpoint_out, endpoint_in, text)
    return 0


def cmd_at(device, args):
    endpoint_out, endpoint_in = at_port(device, args.interface)
    for text in args.commands:
        command(device, endpoint_out, endpoint_in, text)
    return 0


def cmd_dump(device, args):
    log(f"bus {device.bus} addr {device.address} "
        f"configurations {device.bNumConfigurations}")
    for index, config in enumerate(device):
        log(f"config[{index}] value={config.bConfigurationValue} "
            f"interfaces={config.bNumInterfaces} length={config.wTotalLength}")
        for interface in config:
            log(f"   iface {interface.bInterfaceNumber} alt {interface.bAlternateSetting} "
                f"class {interface.bInterfaceClass:02x}:{interface.bInterfaceSubClass:02x}:"
                f"{interface.bInterfaceProtocol:02x}")
            for endpoint in interface:
                log(f"      ep {endpoint.bEndpointAddress:02x} "
                    f"attr {endpoint.bmAttributes:02x} "
                    f"maxp {endpoint.wMaxPacketSize} interval {endpoint.bInterval}")
    return 0


def mbim_send(device, data):
    return device.ctrl_transfer(0x21, 0x00, 0, data[0], data[1], timeout=5000)


def mbim_fetch(device, interface, size=4096):
    return bytes(device.ctrl_transfer(0xA1, 0x01, 0, interface, size, timeout=5000))


def mbim_notify(device, endpoint, seconds):
    if not endpoint:
        return b""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            data = device.read(endpoint, 64, timeout=500)
            if data:
                return bytes(data)
        except usb.core.USBTimeoutError:
            continue
        except usb.core.USBError:
            return b""
    return b""


def mbim_fetch_for(device, interface, endpoint, transaction, tries=5):
    for _ in range(tries):
        mbim_notify(device, endpoint, 2.0)
        response = mbim_fetch(device, interface)
        if len(response) >= 12:
            current = struct.unpack("<I", response[8:12])[0]
            log(f"rx txn={current}: {response.hex()}")
            if current == transaction:
                return response
    return b""


def cmd_mbim_raw(device, args):
    service = bytes.fromhex(args.service.replace("-", ""))
    interface = args.interface
    entry = usb.util.find_descriptor(
        device.get_active_configuration(), bInterfaceNumber=interface)
    if entry is None:
        raise SystemExit(f"interface {interface} not found")
    notify = next((ep.bEndpointAddress for ep in entry
                   if ep.bmAttributes & 0x03 == usb.util.ENDPOINT_TYPE_INTR
                   and ep.bEndpointAddress & 0x80), 0)
    usb.util.claim_interface(device, interface)

    # 排空上次运行遗留的响应，再发一条 MBIM OPEN（12 字节头 + MaxControlTransfer）。
    for _ in range(6):
        mbim_notify(device, notify, 0.3)
        if not mbim_fetch(device, interface):
            break
    device.ctrl_transfer(0x21, 0x00, 0, interface, struct.pack("<IIII", 1, 16, 1, 4096),
                         timeout=5000)
    log(f"open: {mbim_fetch_for(device, interface, notify, 1).hex()}")

    payload = struct.pack("<II", 1, 0) + service + struct.pack("<III", args.cid, 0, 0)
    message = struct.pack("<II", 3, 12 + len(payload)) + struct.pack("<I", 2) + payload
    log(f"tx: {message.hex()}")
    device.ctrl_transfer(0x21, 0x00, 0, interface, message, timeout=5000)
    response = mbim_fetch_for(device, interface, notify, 2)
    if len(response) < 48:
        log(f"short response: {response.hex()}")
        return 1
    _, _, _ = struct.unpack("<III", response[:12])
    _, _ = struct.unpack("<II", response[12:20])
    cid, status, length = struct.unpack("<III", response[36:48])
    info = response[48:48 + length]
    log(f"rx cid={cid} status={status} info_len={length} data={info.hex()}")
    if len(info) >= 8:
        first, second = struct.unpack("<II", info[:8])
        for order, offset, size in (("offset,length", first, second),
                                    ("length,offset", second, first)):
            if offset + size <= len(info):
                log(f"   {order}: {info[offset:offset + size].hex()}")
    return 0


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--interface", type=int, default=0)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("status")
    dump = subparsers.add_parser("dump")
    send = subparsers.add_parser("at")
    send.add_argument("commands", nargs="+")
    raw = subparsers.add_parser("mbim-raw")
    raw.add_argument("--service", default=UICC_LLA.hex())
    raw.add_argument("--cid", type=int, default=ATR_CID)
    args = parser.parse_args()

    device = find_device()
    if args.command == "dump":
        return cmd_dump(device, args)
    if args.command == "mbim-raw":
        return cmd_mbim_raw(device, args)
    if args.command == "status":
        return cmd_status(device, args)
    return cmd_at(device, args)


if __name__ == "__main__":
    sys.exit(main())
