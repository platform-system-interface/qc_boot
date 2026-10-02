# Qualcomm mask ROM tool

This tool lets you interact with Qualcomm SoCs in
[EDL mode](https://en.wikipedia.org/wiki/Qualcomm_EDL_mode).
Note that Qualcomm has evolved EDL over time.

## Boot flow

Each SoC may have a somewhat different boot flow.
See the [boot flow docs](./docs/boot-flow.md) for details.

## Using this tool

**NOTE**: The `qcserial` kernel module must not be loaded.
TL;DR: `sudo modprobe -r qcserial`

Enter EDL/QDL mode, and connect a USB cable to the device. Depending on the
platform, this may involve pulling some pin to ground while plugging in.
If successful, you should see the device in `lsusb`:

```
Bus 002 Device 014: ID 05c6:9008 Qualcomm, Inc. Gobi Wireless Modem (QDL mode)
```

See also: <https://docs.qualcomm.com/doc/80-70017-254/topic/flash_images.html>

## Hardware Information

```
cargo run --release -- info
```

### TP-Link M7350 v3

|    feature    |         value        |
| ------------- | -------------------- |
| serial number | `78 9e e2 1b`        |
| hardware ID   | `007F10E1` (MDM9225) |

https://clickgsm.ro/software-factory/qualcomm-snapdragon-x5-modem-mdm9225-1--6om/

### TP-Link M7350 v4

|    feature    |         value        |
| ------------- | -------------------- |
| serial number | `a8 ed 62 6b`        |
| hardware ID   | `000480E1` (MDM9207) |

https://clickgsm.ro/software-rootare/qualcomm-snapdragon-x5-modem-mdm9207-c-000480e1-69u/
