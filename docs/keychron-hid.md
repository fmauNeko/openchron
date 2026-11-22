# Keychron HID protocols

What Openchron knows about talking to Keychron devices, and how sure it is. Keychron documents none of this: it comes from captures on real hardware and from the community projects listed under [Sources](#sources).

Status: **verified** means captured on hardware (a Keychron G6 HE mouse paired with a TurboLink 8K receiver, mouse firmware 1.0.6+0, receiver USB release 0x0126), **reported** means read in another project's documentation or code but not captured here yet.

## Devices

All Keychron devices use USB vendor ID `0x3434`. A 2.4 GHz receiver has its own product ID and relays the device paired with it, so its vendor interface answers for the mouse.

| Product ID | Product                | Kind     | Status                                  |
| ---------- | ---------------------- | -------- | --------------------------------------- |
| `0xD028`   | Keychron Ultra-Link 8K | Receiver | reported                                |
| `0xD030`   | Keychron Link          | Receiver | reported                                |
| `0xD031`   | Keychron Link          | Receiver | reported                                |
| `0xD038`   | Keychron 4K Link       | Receiver | reported                                |
| `0xD05A`   | Keychron TurboLink 8K  | Receiver | verified (USB product string)           |
| `0xD048`   | Keychron M5            | Mouse    | reported                                |
| `0xD049`   | Keychron M6 8K         | Mouse    | reported                                |
| `0xD056`   | Keychron M7 8K         | Mouse    | reported                                |
| `0xD060`   | Keychron M6            | Mouse    | reported                                |
| `0xD06E`   | Keychron G3            | Mouse    | reported                                |
| `0xD09D`   | Keychron G6 HE         | Mouse    | verified ID; name inferred from the receiver it ships with |

`src/devices/catalog.rs` holds this table. Unknown products work anyway: they are named after their USB product string.

## Interfaces

Openchron picks interfaces by HID usage page:

| Usage page | Usage  | Protocol                                   | Used for                            |
| ---------- | ------ | ------------------------------------------ | ----------------------------------- |
| `0xFFC1`   | `0x01` | Keychron mouse protocol ("8k" in Launcher) | mice and mouse receivers            |
| `0xFF60`   | `0x61` | VIA raw HID                                | QMK keyboards                       |
| `0xFF0A`   | any    | Keychron "4k" / "8k_nordic" (reported)     | not supported yet                   |
| `0x008C`   | `0x01` | Keychron "1k" receiver bridge (reported)   | not supported yet                   |

The TurboLink 8K exposes three interfaces: `MI_00` (keyboard, consumer, system and mouse input collections, also used by the four buttons of its dock), `MI_01` (`0xFFC1`) and `MI_02` (`0x008C`).

## Mouse protocol (`0xFFC1`)

### Framing (verified)

| Report | Direction | Payload  | Use                         |
| ------ | --------- | -------- | --------------------------- |
| `0xB5` | out       | 20 bytes | short requests              |
| `0xB6` | in        | 63 bytes | replies to `0xB5`, events   |
| `0xB3` | out       | 63 bytes | long requests               |
| `0xB4` | in        | 63 bytes | replies to `0xB3`           |

Byte 0 of every payload is the opcode; the rest is zero-padded. With hidapi, a write starts with the report ID and a read returns the report ID first. Replies repeat the opcode in byte 0, or acknowledge a setter with `E4 <status> <opcode>` (status 0 is success). Stale bytes from earlier replies follow the meaningful part of a reply, so parsers read fixed offsets only.

Keychron Launcher keeps one request in flight, waits up to 1 s for its answer and sends it up to 3 times. Openchron does the same (`src/devices/session.rs`), and keeps reports that arrive meanwhile as events.

### Requests used by Openchron

| Opcode | Report | Request                       | Answer                                                                                                     | Status   |
| ------ | ------ | ----------------------------- | ---------------------------------------------------------------------------------------------------------- | -------- |
| `0x02` | `B5`   | `02 <mode>`                   | `02 <protocol u16> <vid u16> <pid u16> <fw minor:patch> <fw major> <mode> _ <feat1> <feat2> <feat3> _ <feat4>` | verified |
| `0x03` | `B5`   | `03`                          | `03 <n>` then `n` × `<vid u16> <pid u16> <active>`                                                          | verified |
| `0x04` | `B3`   | `04 <mode>`                   | `04 <len> <ASCII version>`                                                                                  | verified |
| `0x06` | `B3`   | `06`                          | base info, below                                                                                           | verified |
| `0x49` | `B3`   | `49`                          | separate X/Y DPI table, below                                                                              | verified |
| `0x48` | `B3`   | the `0x49` table, opcode `48` | `E4 00 48` (reported); Openchron reads the table back                                                      | verified by read-back |
| `0x4B` | `B5`   | `4B`                          | `4B <level wired> <level receiver> <count wired> <count receiver> <6 codes wired> <6 codes receiver>`      | verified |
| `0x4A` | `B5`   | the `0x4B` table, opcode `4A` | acknowledgement not documented; Openchron reads the table back                                            | verified by read-back |
| `0x0E` | `B5`   | `0E <profile> <profile count>` | `E4 00 0E`                                                                                                | reported |
| `0x40` | `B5`   | `40 <stage ×3> <5 × dpi u16> <count>` | `E4` or `40`                                                                                       | reported |
| `0x41` | `B5`   | `41 <level ×2> <6 codes> <count>` | `E4` or `41`                                                                                           | reported |

While the mouse sleeps, the receiver still answers `0x03` itself, listing the mouse as inactive, but relays the other requests to the mouse, which leaves them unanswered (verified). Openchron therefore asks a receiver for `0x03` first.

`<mode>` is the connection: 0 wired, 1 receiver, 2 Bluetooth. Multi-byte values are little-endian. `0x40` and `0x41` are for mice without separate X/Y DPI (feature 3 bit 0) or per-mode polling rates (feature 3 bit 1). Feature flags come from `0x02` from protocol 6 on, and from the base info (bytes 26, 53, 60) before. Feature 1 bit 5 means profiles. Byte 2 of `0x0E` also sets the profile count, so it must repeat the current count.

Base info (`0x06`) layout: 1 profile, 2–4 one byte per mode (DPI stage in the low nibble, polling level in the high one), 5–14 five DPI values, 16 DPI stage count, 17 debounce (ms), 18 sleep timeout (minutes), 19 battery (bits 0–6 percent, bit 7 charging), 43–48 polling-rate codes, 49 code count (0 means 6), 50 profile count.

DPI table (`0x49`), protocol 6 and later: 1–3 active stage per mode, 4 stage count, 5–20 eight X values, 21–36 eight Y values, 37 flags. Before protocol 6, each mode byte packs the Y stage over the X stage, and 37–44 hold one flag per stage.

Polling-rate codes: `0` 125 Hz, `6` 250 Hz, `1` 500 Hz, `2` 1000 Hz, `3` 2000 Hz, `4` 4000 Hz, `5` 8000 Hz.

Openchron selects a DPI stage in every connection mode at once (like Launcher's `0x40`), and a polling rate for the current connection only.

### Events (verified unless noted)

| Opcode | Event          | Payload                                                                                             |
| ------ | -------------- | --------------------------------------------------------------------------------------------------- |
| `0xE2` | status changed | `<mode> <connected> <power> <battery %> <DPI stage> <polling level> <DPI count> <polling count>`   |
| `0xE5` | profile changed (reported) | `<profile>`                                                                             |

The receiver sends `0xE2` on every press of a DPI Cycle button (Keychron Launcher assigns that function to a button), with the new stage, and when the mouse wakes up. On waking, the first `0xE2` has every field after `connected` at 0, the DPI count included. Openchron updates the battery, DPI stage and polling level from `0xE2`, and reads everything again when the mouse reconnects, the DPI count is 0, `power` changes or a profile changes. The meaning of the `power` values is not known yet: a change is only a cue to read the charging bit of the base info.

### Captures

G6 HE through the TurboLink 8K, report ID first:

```text
OUT B5 02 01  IN B6 02 06 00 34 34 9D D0 06 01 59 00 BB 9F 9B 00 38 01
OUT B5 03     IN B6 03 01 34 34 9D D0 01
OUT B3 04 01  IN B4 04 07 31 2E 30 2E 36 2B 30                      "1.0.6+0"
OUT B3 06     IN B4 06 00 22 22 02 90 01 20 03 40 06 80 0C 00 00 01 05 04 0A 3B … 50 C3 01 00 01 02 03 04 05 06 05 …
OUT B3 49     IN B4 49 02 02 02 05 90 01 20 03 40 06 80 0C 88 13 00 … 90 01 20 03 40 06 80 0C 88 13 00 … 1F
OUT B5 4B     IN B6 4B 02 02 06 06 00 01 02 03 04 05 00 01 02 03 04 05
event         IN B6 E2 01 01 00 3B 02 02 05 06
```

Protocol 6, 59 % on battery, DPI stages 400/800/1600/3200/5000 with 1600 active, 125 Hz to 8000 Hz with 1000 Hz active, 5 profiles. `src/protocol/mouse.rs` tests replay these frames.

The same receiver while the mouse sleeps, then events as it wakes up and its DPI Cycle button is pressed:

```text
OUT B5 03     IN B6 03 01 34 34 9D D0 00                            mouse paired, inactive
OUT B5 02 01  (no answer within 1.5 s; same for 04, 06, 49 and 4B)
event         IN B6 E2 01 01 00 00 00 00 00                         wake-up: no settings yet
event         IN B6 E2 01 01 00 3B 03 02 05 06                      DPI Cycle: stage 3
event         IN B6 E2 01 01 00 3B 04 02 05 06                      stage 4
event         IN B6 E2 01 01 00 3B 00 02 05 06                      stage 0, the cycle wraps
```

## Keyboards (`0xFF60`)

VIA raw HID: 32-byte reports without a report ID (written after a `0x00` report ID). Openchron sends `id_get_protocol_version` (`01`) and reads `01 <version u16 big-endian>` to identify the keyboard. Keychron's QMK firmware adds its own commands from `0xA0` (firmware version, features, wireless settings); none is used yet.

Keychron keyboards do not report their battery level through a 2.4 GHz receiver, nor through their wired raw HID commands (reported by InputScout for a K8 HE on firmware 1.1.1). Over Bluetooth, the operating system reads the standard GATT Battery Service.

## Sources

- [YouHusam/kc-control](https://github.com/YouHusam/kc-control) (MIT): `ref/keychron-m6-hid-api.md`, the mouse protocol as read in the Keychron Launcher web bundle, with an M6 capture.
- [mammadow/keychron-battery-tray](https://github.com/mammadow/keychron-battery-tray) (MIT): battery through `0xB3 06` and paired devices through `0xB5 03`, on the Ultra-Link 8K.
- [csutcliff/keychron-battery-dkms](https://github.com/csutcliff/keychron-battery-dkms): Linux kernel module reading the same reports.
- [SPDG/inputscout](https://github.com/SPDG/inputscout): read-only status of Keychron mice and keyboards on Linux.
- [the-via/keyboards](https://github.com/the-via/keyboards) and QMK `quantum/via.h`: the VIA protocol.
