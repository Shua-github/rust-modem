#!/usr/bin/env python3
"""Talk AT to a Quectel module over raw USB from WSL, and manage its USB modes.

Subcommands:
  at <命令...>        直接发 AT 命令（可多条）
  status              ATI + usbnet + usbcfg + QADBKEY
  set-usbnet <0|1|2|3> 0=QMI/RmNet, 2=MBIM, 3=RNDIS（切完设备会重新枚举）
  set-usbcfg <bits>   例如 "1,1,1,1,1,1,0"（8 项：vid,pid,diag,nmea,at,modem,rmnet,adb?）
                      传 "adb" 表示读回当前值并把 ADB 位（倒数第二）置 1
  unlock-adb          取码 → 算密钥 → AT+QADBKEY=... → 打开 ADB 位
  reboot              AT+CFUN=1,1

`VID`/`PID` 环境变量可覆盖默认的模组 id（默认 2c7c:0127）。
密钥算法：openssl passwd -1 -salt <码> SH_adb_quectel 取第 12~26 位。
"""

import argparse
import os
import re
import subprocess
import sys
import time

import usb.core
import usb.util

VENDOR = int(os.environ.get("VID", "2c7c"), 16)
PRODUCT = int(os.environ.get("PID", "0127"), 16)


def log(message=""):
    print(message, flush=True)


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


def find_at_port(device):
    """The AT port is the vendor interface that answers a plain AT."""
    for interface in device.get_active_configuration():
        out, inp = bulk_endpoints(interface)
        if not out or not inp:
            continue
        number = interface.bInterfaceNumber
        try:
            usb.util.claim_interface(device, number)
        except usb.core.USBError:
            continue
        try:
            device.write(out, b"AT\r\n", timeout=1000)
            time.sleep(0.3)
            answer = bytes(device.read(inp, 256, timeout=1000))
        except usb.core.USBError:
            usb.util.release_interface(device, number)
            continue
        if b"OK" in answer:
            log(f"AT port: interface {number}, out 0x{out:02x}, in 0x{inp:02x}")
            return out, inp
        usb.util.release_interface(device, number)
    raise SystemExit("no AT port found")


def command(device, endpoint_out, endpoint_in, text, wait=1.5):
    device.write(endpoint_out, (text + "\r\n").encode(), timeout=2000)
    deadline = time.monotonic() + wait
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
    answer = b"".join(chunks).decode("ascii", "replace").strip()
    log(f"> {text}\n{answer}")
    return answer


def unlock_key(code):
    digest = subprocess.run(
        ["openssl", "passwd", "-1", "-salt", code, "SH_adb_quectel"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    return digest[12:27]


def current_usbcfg(answer):
    match = re.search(r'"usbcfg",([0-9A-Fa-fx,]+)', answer)
    if not match:
        raise SystemExit("could not read usbcfg")
    return match.group(1).split(",")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("status")
    subparsers.add_parser("reboot")
    unlock = subparsers.add_parser("unlock-adb")
    unlock.add_argument("--usbcfg", help="解锁后写入的 usbcfg，默认只把 ADB 位置 1")
    send = subparsers.add_parser("at")
    send.add_argument("commands", nargs="+")
    mode = subparsers.add_parser("set-usbnet")
    mode.add_argument("value", choices=["0", "1", "2", "3"])
    config = subparsers.add_parser("set-usbcfg")
    config.add_argument("bits", help="如 1,1,1,1,1,1,0；或填 'adb' 只把 ADB 位置 1")
    args = parser.parse_args()

    device = usb.core.find(idVendor=VENDOR, idProduct=PRODUCT)
    if device is None:
        log("module not found")
        return 2
    log(f"module {device.idVendor:04x}:{device.idProduct:04x} "
        f"bus {device.bus} addr {device.address}")
    endpoint_out, endpoint_in = find_at_port(device)

    if args.command == "status":
        for text in ("ATI", 'AT+QCFG="usbnet"', 'AT+QCFG="usbcfg"', "AT+QADBKEY?"):
            command(device, endpoint_out, endpoint_in, text)
        return 0

    if args.command == "at":
        for text in args.commands:
            command(device, endpoint_out, endpoint_in, text)
        return 0

    if args.command == "set-usbnet":
        command(device, endpoint_out, endpoint_in, f'AT+QCFG="usbnet",{args.value}')
        return 0

    if args.command == "reboot":
        command(device, endpoint_out, endpoint_in, "AT+CFUN=1,1")
        return 0

    bits = None
    if args.command == "unlock-adb":
        answer = command(device, endpoint_out, endpoint_in, "AT+QADBKEY?")
        match = re.search(r"\+QADBKEY:\s*(\S+)", answer)
        if not match:
            log("module did not report a QADBKEY code")
            return 3
        key = unlock_key(match.group(1))
        log(f"code = {match.group(1)}, key = {key}")
        command(device, endpoint_out, endpoint_in, f'AT+QADBKEY="{key}"')
        bits = args.usbcfg
    elif args.command == "set-usbcfg" and args.bits != "adb":
        bits = args.bits

    if bits is None:
        fields = current_usbcfg(
            command(device, endpoint_out, endpoint_in, 'AT+QCFG="usbcfg"'))
        fields[-2] = "1"
        bits = ",".join(fields)

    command(device, endpoint_out, endpoint_in, f'AT+QCFG="usbcfg",{bits}')
    command(device, endpoint_out, endpoint_in, 'AT+QCFG="usbcfg"')
    log("reboot the module (quectel.py reboot) to apply")
    return 0


if __name__ == "__main__":
    sys.exit(main())
