Unofficial native LINE client for Linux. GTK4 / Libadwaita UI with a Deno protocol sidecar ([linejs](https://github.com/evex-dev/linejs)).


![main](./assets/761546283_1013837091419784_4898538377611644175_n.jpg)

---

## Implementation

- [x] QR login (additional device)
- [x] Chats, messages, stickers, friends
- [x] Photos, video, voice messages, files
- [x] Paste and drag-and-drop attachments
- [x] In-app viewers (image, video, PDF, text)
- [x] Downloads, tray, Thai / English, themes
- [x] Discord RPC
- [ ] One-to-one voice calls (experimental, live LINE interoperability unverified)
- [ ] Account switcher

---

## Requirements

| Dependency | Notes |
| --- | --- |
| Rust | stable toolchain |
| GTK4 + Libadwaita | `pacman -S gtk4 libadwaita` |
| Deno | Build-time protocol compiler; source-tree runs use `deno` or `DENO` |
| ffmpeg / ffprobe | voice record, video thumbs |
| grim + slurp (optional) | screen area capture on supported Wayland compositors |
| Optional | `pdftoppm`, `pdftotext` for PDF preview |

Wayland and X11 both work. Tested mainly on Arch-based desktops.

---

## Install (Arch / AUR)

```bash
# Development snapshot
yay -S line-gtk-git

# Tagged release
yay -S line-gtk
```

## Install (Flatpak bundle)
```bash
flatpak install --user ./line-gtk-0.1.2-x86_64.flatpak
flatpak run dev.linegtk.LineGtk
```

## Build and run

```bash
git clone https://github.com/MidnightTale/Line-GTK.git
cd Line-GTK
cargo build --release
./target/release/line-gtk
```

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
deno task --config protocol/deno.json check
deno task --config protocol/deno.json check:call-audio
deno task --config protocol/deno.json check:call-pulse
```

The call tests use an encrypted UDP relay on localhost and real Opus encoding.
`check:call-audio` runs generated audio through FFmpeg, Opus, and FFmpeg playback
to a null output. It does not use your microphone or call another account.
`check:call-pulse` also needs a running PulseAudio-compatible server and `pactl`.
It checks selected-device routing and generated audio through temporary virtual
devices, then removes those devices. It does not capture your physical microphone.
These checks do not establish interoperability with the live LINE service.

---

## Warning

This is an unofficial client. Using it may violate LINE Terms of Service. Prefer a secondary account or device session. Expect breakage when LINE changes login or crypto.

---

## Acknowledgments

Protocol and E2EE talk to LINE through [linejs](https://github.com/evex-dev/linejs) by [evex-dev](https://github.com/evex-dev). This app is a GTK frontend around that library; it is not affiliated with LINE Corporation or evex-dev.

---

## License

[GPL-3.0-or-later](LICENSE)

The Capture button beside the voice message button lets you select a screen area while LINE GTK and its image viewers remain visible, then opens the screenshot in the image viewer. Press Escape to cancel selection. Capture requires a Wayland compositor supported by grim and the `grim` and `slurp` programs on the host. Flatpak capture is not currently supported. Screenshots stay local until you explicitly queue and send them.

Use Read QR in the image viewer to read QR codes from the original image. Results support text copying and explicitly opening HTTP or HTTPS links. Links never open automatically.
