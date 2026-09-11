# udev rule install

`colorer set` writes to a HID device, which needs elevated access by
default on Linux. Installing this rule once grants the logged-in seat
session access, so `set` never needs `sudo`. `list`/`show` (discovery) never
need this — they don't open a device handle.

**Status:** `71-colorer.rules` covers the real, hardware-verified devices in
`src/device/hid.rs`'s `IMPLEMENTED_PROTOCOLS`: Razer Ornata V3 (`1532:02a1`,
interface 2), Razer Naga X (`1532:0096`, interface 3), and the Gigabyte RGB
Fusion 2 onboard controller (`048d:5711`, interface 1, CPU ARGB strip only —
see `gigabyte_fusion2_cpu_strip_report`'s doc comment for that device's scope
caveat). Each line matches `ATTRS{bInterfaceNumber}` too, scoped to the exact
interface `colorer` writes to — not just `idVendor`/`idProduct`, which would
grant `uaccess` to every hidraw interface the device exposes (e.g. a
keyboard's plain input interface alongside its vendor control interface).
Add a new line, mirroring the existing ones (vendor/product/interface all
three), when a further device is reverse-engineered and added to that table.

Note: on at least one dev machine, the Naga X's `hidraw` nodes did **not**
pick up `uaccess` from this rule via a live `udevadm trigger` alone (nor from
unplug/replug) — a full `udevadm control --reload-rules && udevadm trigger`
after the rule file was actually in place was what worked. If a replug alone
doesn't grant access, re-run the reload+trigger step below.

## Install steps

1. Copy the rules file:
   ```sh
   sudo cp packaging/udev/71-colorer.rules /etc/udev/rules.d/
   ```
2. Reload and re-trigger udev:
   ```sh
   sudo udevadm control --reload-rules && sudo udevadm trigger
   ```
3. Replug the target device if it doesn't pick up the new rule immediately.

## Verifying it actually worked

`TAG+="uaccess"` grants access to the current seat session via
systemd-logind — it is **not** a permanent standing grant independent of
session state (it can require a fresh login/session, and doesn't apply the
same way over e.g. SSH with no logind seat). Check actual effective access,
not just that the rule file is present:

```sh
ls -l /dev/hidraw<N>   # confirm the device node's ACL includes your user
cargo run -- set <id> ff0000   # the real test: does the write succeed
```
