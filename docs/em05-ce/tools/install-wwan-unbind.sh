#!/usr/bin/env bash
# 永久解绑 QMI/MBIM 内核驱动（qmi_wwan / cdc_mbim），让 userspace 能独占接口。
#
#   sudo bash install-wwan-unbind.sh            # 安装
#   sudo bash install-wwan-unbind.sh uninstall  # 卸载
#
# 两道保险：
#   /etc/modprobe.d/99-wwan-nobind.conf  —— 阻止模块加载（blacklist + install /bin/false）
#   /etc/udev/rules.d/99-wwan-unbind.rules —— 模块已在内存时，绑定后立刻 unbind

set -euo pipefail

modprobe_conf=/etc/modprobe.d/99-wwan-nobind.conf
udev_rules=/etc/udev/rules.d/99-wwan-unbind.rules

if [ "${1:-install}" = "uninstall" ]; then
    rm -f "$modprobe_conf" "$udev_rules"
    udevadm control --reload-rules 2>/dev/null || true
    echo "已移除 $modprobe_conf 与 $udev_rules（重新插拔/重启后内核恢复接管）"
    exit 0
fi

cat > "$modprobe_conf" <<'EOF'
# 让 QMI / MBIM 接口留给 userspace（WebUSB、rm-cli、libqmi 等）。
blacklist qmi_wwan
blacklist cdc_mbim

# blacklist 只挡按别名的自动加载，install 连显式加载一起挡。
install qmi_wwan /bin/false
install cdc_mbim /bin/false

# 需要连串口/诊断口也留给 userspace 时再打开：
# blacklist option
# blacklist qcserial
# install option /bin/false
# install qcserial /bin/false
EOF

cat > "$udev_rules" <<'EOF'
# 模块已在内存里时（例如别的设备先触发加载），绑定后立刻解绑。
# add 处理“模块已在、设备后插”；bind 处理“设备已在、模块后加载”。
ACTION=="add|bind", SUBSYSTEM=="usb", DRIVERS=="qmi_wwan", RUN+="/bin/sh -c 'echo -n %k > /sys/bus/usb/drivers/qmi_wwan/unbind'"
ACTION=="add|bind", SUBSYSTEM=="usb", DRIVERS=="cdc_mbim", RUN+="/bin/sh -c 'echo -n %k > /sys/bus/usb/drivers/cdc_mbim/unbind'"
EOF

udevadm control --reload-rules 2>/dev/null || true
modprobe -r qmi_wwan cdc_mbim 2>/dev/null || true

echo "已写入："
echo "  $modprobe_conf"
echo "  $udev_rules"
echo "验证：modprobe -n -v qmi_wwan 应输出 install /bin/false"
