# MBIM 的 UICC 参考字节数组：[length][offset]（ATR 截断的根因）

## 现象

`rm-cli mbim uim state` 返回的 ATR 被截断成 8 字节：

```
"atr": [86, 7, 98, 224, 39, 3, 0, 132]        // 0x56 0x07 0x62 0xe0 0x27 0x03 0x00 0x84
```

而完整 ATR（QMI 路径返回的）是 23 字节：

```
3b 9f 96 80 3f c7 83 80 31 e0 73 f6 21 13 67 56 07 62 e0 27 03 00 84
```

截断出来的 8 字节正好是完整 ATR 的**最后 8 字节**。

## 原始报文

用 `docs/other/tools/win/modem.py mbim-raw` 抓 `MBIM_CID_UICC_ATR`
的响应（Windows + WinUSB 绑定 MBIM 控制接口 MI_01）：

```
info: 17 00 00 00 | 08 00 00 00 | 3b 9f 96 80 3f c7 83 80 31 e0 73 f6 21 13 67 56 07 62 e0 27 03 00 84 00
      └─ 23 ─┘      └─ 8 ─┘      └───────────────── 23 字节 ATR ─────────────────┘
```

也就是设备把这对字写成了 **[length=23][offset=8]**，数据在偏移 8、长度 23。

## 根因

MBIM 里有两类“引用型字节数组”：

| 格式 | 线上布局 |
| --- | --- |
| `byte-array` / `ref-byte-array` / `unsized-byte-array` | **[offset][length]** + 数据在尾部 |
| `uicc-ref-byte-array` | **[length][offset]** + 数据在尾部 |

依据是 libmbim 的构造器参数 `swapped_offset_length`
（`ref/libmbim/src/libmbim-glib/mbim-message.c:1307` 起）：

```c
/* (b) Just length in static buffer, data just afterwards. */
if (swapped_offset_length && with_length) {
    length = GUINT32_TO_LE (buffer_len);
    g_byte_array_append (builder->fixed_buffer, (guint8 *)&length, sizeof (length));
}
```

`uicc-ref-byte-array` 共 5 处，全部在 `mbim-service-ms-uicc-low-level-access.json`
（ATR、AppId、APDU 的 command/response 等）。我们的 codegen 之前把它和普通 byte-array
一视同仁，于是按 `[offset=23][length=8]` 去切片，取到 `info[23..31]` —— 正好是那 8 个字节。

## 修复

- `rm-mbim/src/codec.rs`：新增 `Encoder::write_ref_swapped` / `Decoder::read_ref_swapped`
  （先 length 后 offset，数据仍在尾部；写回时只回填第二个字的 offset）。
- `xtask/src/mbim.rs`：`uicc-ref-byte-array` 单独分支，改用上面两个方法；其它格式保持原样。
- 重新生成 `cargo xtask mbim-codegen`（5 个字段都换成了 `read_ref_swapped` / `write_ref_swapped`），
  并同步了手写测试 `rm-mbim/tests/generated.rs` 里的两处期望字节。

## 验证

```
> target\debug\rm-cli.exe mbim uim state
{ "protocol": "mbim", "state": { "1": {
    "atr": [59, 159, 150, 128, 63, 199, 131, 128, 49, 224, 115, 246, 33, 19, 103,
            86, 7, 98, 224, 39, 3, 0, 132],       // 23 字节，0x3B 开头，正确
    "imei": "<imei>", "present": true, "ready": true } } }
```

`cargo test -p rm-mbim` 5 项全过。浏览器壳子（wasm）也已重新构建，WebUSB 路径使用同一份修复。
