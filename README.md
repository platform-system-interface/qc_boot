# Qualcomm mask ROM tool

This tool lets you interact with Qualcomm SoCs in
[EDL mode](https://en.wikipedia.org/wiki/Qualcomm_EDL_mode).

**NOTE**: The `qcserial` must not be loaded; TL;DR: `sudo modprobe -r qcserial`

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
