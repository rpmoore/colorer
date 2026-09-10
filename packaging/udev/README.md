# udev rule install

`colorer set` writes to a HID device, which needs elevated access by
default on Linux. Installing this rule once grants the logged-in seat
session access, so `set` never needs `sudo`. `list`/`show` (discovery) never
need this — they don't open a device handle.

**Status:** `71-colorer.rules` is a placeholder. No real target device has
been identified/reverse-engineered yet (see
`docs/plans/sections/section-05-set-hid.md`), so `<target-vid>`/`<target-pid>`
in the rule file are not filled in, and `colorer set` currently returns
`Unsupported` for every device regardless of whether this rule is installed.
Fill in the real `idVendor`/`idProduct` once a target device is chosen.

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
