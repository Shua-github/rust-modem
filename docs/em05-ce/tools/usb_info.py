#!/usr/bin/env python3
"""Inspect and unbind USB devices from WSL.

Subcommands:
  report                每台设备的接口、类、绑定的内核驱动
  endpoints             每台设备的接口 + 端点（接口号、类、方向、包长）
  config [vid] [pid]    打印配置描述符并重试 SET_CONFIGURATION
  raw-config [vid] [pid] 用递增超时直接发 SET_CONFIGURATION，判断设备是否只是响应慢
  unbind <vid>          解绑某厂商（十六进制，如 05c6）所有接口上的内核驱动

`--all` 会连根集线器一起打印。
"""

import argparse
import glob
import os
import sys
import time

import usb.core
import usb.util

ROOT_HUBS = {"1d6b"}
SET_CONFIGURATION = 0x09


def devices(include_hubs=False):
    for path in sorted(glob.glob("/sys/bus/usb/devices/*")):
        try:
            with open(os.path.join(path, "idVendor")) as handle:
                vendor = handle.read().strip()
        except OSError:
            continue
        if not include_hubs and vendor in ROOT_HUBS:
            continue
        with open(os.path.join(path, "idProduct")) as handle:
            product = handle.read().strip()
        try:
            with open(os.path.join(path, "product")) as handle:
                name = handle.read().strip()
        except OSError:
            name = ""
        yield path, vendor, product, name


def read(path, name, default=""):
    try:
        with open(os.path.join(path, name)) as handle:
            return handle.read().strip()
    except OSError:
        return default


def driver_of(interface):
    link = os.path.join(interface, "driver")
    if not os.path.islink(link):
        return ""
    return os.path.basename(os.readlink(link))


def report(include_hubs):
    for path, vendor, product, name in devices(include_hubs):
        print(f"== {os.path.basename(path)} {vendor}:{product} {name}")
        for interface in sorted(glob.glob(os.path.join(path, ":*"))):
            if not os.path.exists(os.path.join(interface, "bInterfaceClass")):
                continue
            print(f"   {os.path.basename(interface):<10} "
                  f"class={read(interface, 'bInterfaceClass')}:"
                  f"{read(interface, 'bInterfaceSubClass')}:"
                  f"{read(interface, 'bInterfaceProtocol')} "
                  f"driver={driver_of(interface) or 'none'}")


def endpoints(include_hubs):
    for path, vendor, product, name in devices(include_hubs):
        print(f"== {os.path.basename(path)} {vendor}:{product} {name}")
        for interface in sorted(glob.glob(os.path.join(path, ":*"))):
            if not os.path.exists(os.path.join(interface, "bInterfaceClass")):
                continue
            print(f"   iface {os.path.basename(interface)} "
                  f"class={read(interface, 'bInterfaceClass')}:"
                  f"{read(interface, 'bInterfaceSubClass')}:"
                  f"{read(interface, 'bInterfaceProtocol')} "
                  f"altsetting={read(interface, 'bAlternateSetting')}")
            for endpoint in sorted(glob.glob(os.path.join(interface, "ep_*"))):
                if not os.path.exists(os.path.join(endpoint, "type")):
                    continue
                print(f"      {os.path.basename(endpoint):<8} "
                      f"type={read(endpoint, 'type'):<10} "
                      f"dir={read(endpoint, 'direction'):<3} "
                      f"maxp={read(endpoint, 'wMaxPacketSize'):<5} "
                      f"interval={read(endpoint, 'bInterval')}")


def find_device(vendor, product):
    identifier = int(vendor, 16)
    number = int(product, 16) if product else None
    device = usb.core.find(idVendor=identifier, idProduct=number)
    if device is None:
        print("device not found")
    return device


def show_config(device, reset):
    print(f"device {device.idVendor:04x}:{device.idProduct:04x} "
          f"bus {device.bus} addr {device.address} "
          f"configurations {device.bNumConfigurations}")
    for index, config in enumerate(device):
        print(f"config[{index}] value={config.bConfigurationValue} "
              f"interfaces={config.bNumInterfaces} length={config.wTotalLength}")
        for interface in config:
            print(f"   iface {interface.bInterfaceNumber} "
                  f"alt {interface.bAlternateSetting} "
                  f"class {interface.bInterfaceClass:02x}:"
                  f"{interface.bInterfaceSubClass:02x}:"
                  f"{interface.bInterfaceProtocol:02x}")
            for endpoint in interface:
                print(f"      ep {endpoint.bEndpointAddress:02x} "
                      f"attr {endpoint.bmAttributes:02x} "
                      f"maxp {endpoint.wMaxPacketSize} "
                      f"interval {endpoint.bInterval}")

    if reset:
        try:
            device.reset()
            print("reset -> ok")
        except usb.core.USBError as error:
            print(f"reset -> {error}")

    try:
        device.set_configuration()
        print("set_configuration -> ok")
    except usb.core.USBError as error:
        print(f"set_configuration -> {error}")


def raw_config(device):
    for timeout in (5000, 15000, 30000):
        start = time.monotonic()
        try:
            device.ctrl_transfer(0x00, SET_CONFIGURATION, 1, 0, None, timeout=timeout)
            print(f"SET_CONFIGURATION timeout={timeout}ms -> ok "
                  f"in {time.monotonic() - start:.2f}s")
            return 0
        except usb.core.USBError as error:
            print(f"SET_CONFIGURATION timeout={timeout}ms -> {error} "
                  f"after {time.monotonic() - start:.2f}s")
    return 1


def unbind(vendor):
    vendor = vendor.lower()
    found = False
    for path, device_vendor, _, name in devices():
        if device_vendor != vendor:
            continue
        found = True
        print(f"== {os.path.basename(path)} {name}")
        for interface in sorted(glob.glob(os.path.join(path, ":*"))):
            if not os.path.exists(os.path.join(interface, "bInterfaceClass")):
                continue
            driver = driver_of(interface)
            name = os.path.basename(interface)
            if not driver:
                print(f"   {name} already unbound")
                continue
            target = f"/sys/bus/usb/drivers/{driver}/unbind"
            try:
                with open(target, "w") as handle:
                    handle.write(name)
                print(f"   {name} unbound from {driver}")
            except OSError as error:
                print(f"   {name} FAILED to unbind from {driver}: {error}")
    if not found:
        print(f"no device with vendor id {vendor} is attached")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--all", action="store_true", help="include root hubs")
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("report")
    subparsers.add_parser("endpoints")

    for name in ("config", "raw-config"):
        sub = subparsers.add_parser(name)
        sub.add_argument("vid", nargs="?", default="2c7c")
        sub.add_argument("pid", nargs="?", default="0127")
        if name == "config":
            sub.add_argument("--reset", action="store_true")

    unbound = subparsers.add_parser("unbind")
    unbound.add_argument("vid", nargs="?", default="05c6")

    args = parser.parse_args()

    if args.command == "report":
        report(args.all)
    elif args.command == "endpoints":
        endpoints(args.all)
    elif args.command == "unbind":
        unbind(args.vid)
    else:
        device = find_device(args.vid, args.pid)
        if device is None:
            return 2
        if args.command == "config":
            show_config(device, args.reset)
        else:
            return raw_config(device)
    return 0


if __name__ == "__main__":
    sys.exit(main())
