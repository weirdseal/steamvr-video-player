# SteamVR Video Player

[한국어](README.ko.md)

A Rust video player for Windows 10 and 11 x64 that sends video directly to SteamVR through OpenXR. Desktop playback requires OpenGL 3.3 or newer.

## Build and run

Install Rust 1.88 or newer with the Windows x64 MSVC toolchain, Visual Studio C++ Build Tools, and the Windows SDK. Run these commands from the repository root in PowerShell.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build.ps1
& .\target\x86_64-pc-windows-msvc\release\steamvr-video-player.exe
```

If MinGW-w64 GCC is on `PATH`, you can use the GNU toolchain.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build.ps1 -Gnu
& .\target\x86_64-pc-windows-gnu\release\steamvr-video-player.exe
```

The build script places `libmpv-2.dll` and `openxr_loader.dll` beside the executable. Keep both DLLs with it.

## Desktop playback

Press `Ctrl+O` or drag a video into the window. The player opens formats supported by libmpv, including MP4, MKV, MOV, and AVI.

| Shortcut | Action |
| --- | --- |
| `Space` | Play or pause |
| `←` / `→` | Skip backward or forward |
| `↑` / `↓` / `M` | Adjust volume or mute |
| `F` / `F11` / `Esc` | Toggle or exit fullscreen |
| `V` | Enter or exit VR |

Set separate PC and VR skip intervals from 1 to 60 seconds in **Settings → Playback**. The defaults are 5 seconds on PC and 10 seconds in VR.

## SteamVR

1. Connect your headset, start SteamVR, and select SteamVR as the active OpenXR runtime. Use **Check headset** to verify the connection.
2. Under **Settings → VR**, choose Flat, 180°, or 360° projection and Mono, SBS, or Top/Bottom stereo. Select **Enter VR**. Playback continues from the same position when switching modes.
3. Close the SteamVR dashboard to use the player controls. Press the trigger to show the playback panel. Its folder button opens a sortable browser with list and thumbnail grid views.

| Controller | Action |
| --- | --- |
| Trigger tap | Select a control or open a file. Point outside the panels and tap to dismiss. |
| Trigger drag | Seek on the timeline or scroll through files |
| Stick up or down | Zoom, or scroll while pointing at the browser |
| Stick left or right | Skip by the VR interval |
| Stick click | Recenter the view and reset zoom |
| Grip + stick | Rotate the view |

[Development conventions](CLAUDE.md) · [Noto Sans KR license](assets/OFL-NotoSansKR.txt)

The player source code is licensed under the WTFPL v2. [License text](LICENSE)
