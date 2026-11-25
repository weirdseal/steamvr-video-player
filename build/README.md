# Build and package

Use Rust 1.88 or newer. Run the commands from the repository root in PowerShell.

## MSVC

Install the Windows x64 MSVC toolchain, Visual Studio C++ Build Tools, and the Windows SDK.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build\build.ps1
& .\target\x86_64-pc-windows-msvc\release\steamvr-video-player.exe
```

## GNU

Put MinGW-w64 GCC and `windres` on `PATH`.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
powershell -NoProfile -ExecutionPolicy Bypass -File .\build\build.ps1 -Gnu
& .\target\x86_64-pc-windows-gnu\release\steamvr-video-player.exe
```

The build script places `libmpv-2.dll` and `openxr_loader.dll` beside the executable. Keep both DLLs with it.

## Portable ZIP

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build\package.ps1
```

Add `-Gnu` to select the GNU toolchain. Use `-SkipBuild` to package an existing release build. The script creates `steamvr-video-player-windows-x64.zip` in the release folder.

Extract the ZIP once and run `steamvr-video-player.exe` from the extracted `steamvr-video-player` folder. Keep the DLLs beside the executable. VR mode requires SteamVR.
