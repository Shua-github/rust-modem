#!/usr/bin/env python3
"""Step-by-step USB probes for CDC WDM modems, from WSL.

Subcommands:
  qmi  [--set-config] [--dtr] [--request]   在厂商接口上复现 QMI 流程：
        claim →（可选 SET_CONFIGURATION / SET_CONTROL_LINE_STATE / 发 QMI GetClientId）
        → 读中断通知 → GET_ENCAPSULATED_RESPONSE
  mbim [--set-config] [--data-alt N] [--open] [--notify] [--fetch]
        claim →（可选 SET_CONFIGURATION / 数据接口 altsetting）→ MBIM OPEN
        → 读通知 → 取响应
  find-qmi [--dtr]   逐个厂商接口试哪一个是 QMI 控制口

`IFACE`/`VID`/`PID` 环境变量可覆盖默认（接口 2、2c7c:0127）。
"""

import argparse
import os
import struct
import sys
import time

import usb.core
import usb.util

VENDOR = int(os.environ.get("VID", "2c7c"), 16)
PRODUCT = int(os.environ.get("PID", "0127"), 16)


def log(message):
    print(message, flush=True)


def find_device():
    device = usb.core.find(idVendor=VENDOR, idProduct=PRODUCT)
    if device is None:
        raise SystemExit("device not found")
    log(f"device {device.idVendor:04x}:{device.idProduct:04x} "
        f"bus {device.bus} addr {device.address}")
    return device


def get_client_id_request(service=0x0B, transaction=1):
    """QMUX frame carrying a QMI CTL GetClientId request."""
    tlv = bytes([0x01, 0x04, service, 0x00, 0x00, 0x00])
    qmi = bytes([0x00, len(tlv) & 0xFF, len(tlv) >> 8]) + tlv
    body = bytes([0x00, 0x00, 0x00, transaction & 0xFF, transaction >> 8]) + qmi
    return bytes([0x01, len(body) & 0xFF, len(body) >> 8]) + body


def interface_endpoints(interface):
    out = inp = int_in = 0
    for endpoint in interface:
        address = endpoint.bEndpointAddress
        kind = endpoint.bmAttributes & 0x03
        if kind == usb.util.ENDPOINT_TYPE_BULK:
            if address & 0x80:
                inp = address
            else:
                out = address
        elif kind == usb.util.ENDPOINT_TYPE_INTR and address & 0x80:
            int_in = address
    return out, inp, int_in


def read_notification(device, endpoint, timeout=3000):
    if not endpoint:
        log("   notification -> no interrupt endpoint")
        return
    try:
        data = device.read(endpoint, 64, timeout=timeout)
        log(f"   notification -> {bytes(data).hex()}")
    except usb.core.USBTimeoutError:
        log("   notification -> timeout")
    except usb.core.USBError as error:
        log(f"   notification -> {error}")


def qmi_command(device, interface, request):
    usb.util.claim_interface(device, interface)
    try:
        sent = device.ctrl_transfer(0x21, 0x00, 0, interface, request, timeout=3000)
        log(f"   SEND_ENCAPSULATED_COMMAND -> {sent} bytes")
    except usb.core.USBError as error:
        log(f"   SEND_ENCAPSULATED_COMMAND -> {error}")
        usb.util.release_interface(device, interface)
        return
    try:
        data = bytes(device.ctrl_transfer(0xA1, 0x01, 0, interface, 4096, timeout=3000))
        log(f"   GET_ENCAPSULATED_RESPONSE -> {len(data)} bytes: {data[:32].hex()}")
    except usb.core.USBError as error:
        log(f"   GET_ENCAPSULATED_RESPONSE -> {error}")
    usb.util.release_interface(device, interface)


def probe_qmi(args):
    device = find_device()
    interface = int(os.environ.get("IFACE", args.interface))
    config = device.get_active_configuration()
    entry = usb.util.find_descriptor(config, bInterfaceNumber=interface)
    if entry is None:
        raise SystemExit(f"interface {interface} not found")
    out, inp, int_in = interface_endpoints(entry)
    log(f"iface {interface}: class {entry.bInterfaceClass:02x}:"
        f"{entry.bInterfaceSubClass:02x}:{entry.bInterfaceProtocol:02x} "
        f"bulk_out={out} bulk_in={inp} intr_in={int_in}")

    if args.set_config:
        try:
            device.set_configuration()
            log("SET_CONFIGURATION -> ok")
        except usb.core.USBError as error:
            log(f"SET_CONFIGURATION -> {error}")

    if args.dtr:
        try:
            device.ctrl_transfer(0x21, 0x22, 1, interface, None, timeout=3000)
            log("SET_CONTROL_LINE_STATE(DTR) -> ok")
        except usb.core.USBError as error:
            log(f"SET_CONTROL_LINE_STATE(DTR) -> {error}")

    if args.request:
        request = get_client_id_request()
        log(f"QMI request: {request.hex()}")
    else:
        request = None

    usb.util.claim_interface(device, interface)
    if request is not None:
        try:
            device.ctrl_transfer(0x21, 0x00, 0, interface, request, timeout=3000)
            log("SEND_ENCAPSULATED_COMMAND -> ok")
        except usb.core.USBError as error:
            log(f"SEND_ENCAPSULATED_COMMAND -> {error}")
    read_notification(device, int_in)
    try:
        data = bytes(device.ctrl_transfer(0xA1, 0x01, 0, interface, 4096, timeout=3000))
        log(f"GET_ENCAPSULATED_RESPONSE -> {len(data)} bytes: {data[:32].hex()}")
    except usb.core.USBError as error:
        log(f"GET_ENCAPSULATED_RESPONSE -> {error}")
    usb.util.release_interface(device, interface)
    return 0


def probe_mbim(args):
    device = find_device()
    interface = int(os.environ.get("IFACE", args.interface))
    config = device.get_active_configuration()
    entry = usb.util.find_descriptor(config, bInterfaceNumber=interface)
    if entry is None:
        raise SystemExit(f"interface {interface} not found")
    _, _, int_in = interface_endpoints(entry)
    log(f"iface {interface}: class {entry.bInterfaceClass:02x}:"
        f"{entry.bInterfaceSubClass:02x}:{entry.bInterfaceProtocol:02x} intr_in={int_in}")

    if args.set_config:
        try:
            device.set_configuration()
            log("SET_CONFIGURATION -> ok")
        except usb.core.USBError as error:
            log(f"SET_CONFIGURATION -> {error}")

    usb.util.claim_interface(device, interface)

    if args.data_alt is not None:
        try:
            usb.util.claim_interface(device, args.data_interface)
            device.set_interface_altsetting(
                interface=args.data_interface, alternate_setting=args.data_alt)
            log(f"data interface {args.data_interface} -> altsetting {args.data_alt}")
        except usb.core.USBError as error:
            log(f"data interface {args.data_interface} -> {error}")

    if args.open:
        message = struct.pack("<III", 1, 12, 1)  # MBIM_OPEN_MSG
        try:
            device.ctrl_transfer(0x21, 0x00, 0, interface, message, timeout=3000)
            log(f"MBIM OPEN ({len(message)} bytes) -> sent")
        except usb.core.USBError as error:
            log(f"MBIM OPEN -> {error}")

    if args.notify:
        read_notification(device, int_in)

    if args.fetch:
        try:
            data = bytes(device.ctrl_transfer(0xA1, 0x01, 0, interface, 4096, timeout=3000))
            log(f"GET_ENCAPSULATED_RESPONSE -> {len(data)} bytes: {data[:32].hex()}")
        except usb.core.USBError as error:
            log(f"GET_ENCAPSULATED_RESPONSE -> {error}")

    usb.util.release_interface(device, interface)
    return 0


def find_qmi(args):
    device = find_device()
    request = get_client_id_request()
    log(f"QMI request: {request.hex()}")
    for interface in device.get_active_configuration():
        out, inp, int_in = interface_endpoints(interface)
        if not out or not inp:
            continue
        number = interface.bInterfaceNumber
        log(f"iface {number}: class {interface.bInterfaceClass:02x}:"
            f"{interface.bInterfaceSubClass:02x}:{interface.bInterfaceProtocol:02x} "
            f"bulk_out={out} bulk_in={inp} intr_in={int_in}")
        try:
            usb.util.claim_interface(device, number)
        except usb.core.USBError as error:
            log(f"   claim -> {error}")
            continue
        if args.dtr:
            try:
                device.ctrl_transfer(0x21, 0x22, 1, number, None, timeout=3000)
                log("   SET_CONTROL_LINE_STATE(DTR) -> ok")
            except usb.core.USBError as error:
                log(f"   SET_CONTROL_LINE_STATE(DTR) -> {error}")
        try:
            device.ctrl_transfer(0x21, 0x00, 0, number, request, timeout=3000)
            log("   SEND_ENCAPSULATED_COMMAND -> ok")
            answer = device.read(int_in, 64, timeout=3000) if int_in else None
            log(f"   notification -> {bytes(answer).hex() if answer is not None else 'none'}")
        except usb.core.USBError as error:
            log(f"   -> {error}")
        usb.util.release_interface(device, number)
        time.sleep(0.3)
    return 0


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subparsers = parser.add_subparsers(dest="command", required=True)

    qmi = subparsers.add_parser("qmi")
    qmi.add_argument("--interface", default="2")
    qmi.add_argument("--set-config", action="store_true")
    qmi.add_argument("--dtr", action="store_true")
    qmi.add_argument("--request", action="store_true")

    mbim = subparsers.add_parser("mbim")
    mbim.add_argument("--interface", default="1")
    mbim.add_argument("--data-interface", type=int, default=2)
    mbim.add_argument("--data-alt", type=int)
    mbim.add_argument("--set-config", action="store_true")
    mbim.add_argument("--open", action="store_true")
    mbim.add_argument("--notify", action="store_true")
    mbim.add_argument("--fetch", action="store_true")

    find = subparsers.add_parser("find-qmi")
    find.add_argument("--dtr", action="store_true")

    args = parser.parse_args()
    if args.command == "qmi":
        return probe_qmi(args)
    if args.command == "mbim":
        return probe_mbim(args)
    return find_qmi(args)


if __name__ == "__main__":
    sys.exit(main())
