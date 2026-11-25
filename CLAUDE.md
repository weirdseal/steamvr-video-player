# SteamVR Video Player Development Conventions

SteamVR Video Player is a Rust desktop video player for Windows 10 and 11 x64. It uses egui, libmpv, and OpenGL with WGL, and sends video to SteamVR through OpenXR. See the [player README](README.md) and the [build guide](build/README.md).

## Code Organization

- `desktop.rs` manages the native window, file picker, and event loop. `ui.rs` and `vr/panel.rs` emit `ui::Action` values, which `desktop.rs` handles to update playback and settings.
- `mpv.rs` manages DLL calls, handle lifetimes, and playback state. `graphics.rs` manages textures shared between the desktop and VR and synchronizes access to them. `vr.rs` owns the OpenXR session and swapchains.
- `vr/input.rs` reads controller state. `vr/panel.rs` handles aiming and trigger gestures. `vr/browser.rs` renders folders and entries. The browser and previous and next video controls share the sort order defined in `files.rs`.
- `settings.rs` defines defaults and the on-disk format. When adding settings, preserve compatibility with existing `settings.json` files through stable field names and suitable defaults. Use `settings_ui.rs` for controls shared by the desktop and VR.
- Use the palette and control surfaces from `theme.rs` in both desktop and VR views.
- Write this file and all code comments in English.

## Localization

- Add user-facing labels, help text, accessibility names, and error messages to both UTF-8 catalogs, `locales/ko.json` and `locales/en.json`, under the same PascalCase key. Edit wording in the relevant language file. `build/build.rs` checks that both catalogs have the same keys and generates the Rust `Text` enum and string tables.
- Resolve UI strings with `language.text(key)` using the view's current `Language`. Use `tr(key)` in error handling and file dialogs that lack an explicit language argument. Give widgets stable IDs independent of language. Keep filenames, shortcut names, upstream error details, and diagnostic logs in their original form.
- Use the shared Noto Sans KR font for player text and numeric readouts in both the desktop and VR views. Check that the embedded font subset covers new localized text.

## Playback and VR

- Use one libmpv playback instance for the desktop and VR. Carry the open file, playback position, and playback state across VR transitions. Run OpenGL operations on the thread where the relevant WGL context is current, and use fences to synchronize reads and writes to shared textures.
- Follow the OpenXR frame order of wait, begin, and submit. Report session and panel errors through VR notifications so desktop playback can continue. Cancel active timeline and browser gestures when the session loses focus.
- Draw only visible browser rows within a fixed-height viewport. List directories and decode thumbnails in the background. `mpv/thumbnail.rs` extracts small preview frames. `vr/thumbnails.rs` schedules previews for visible tiles and manages a bounded texture cache. Use each file's path, modification time, and size to identify cached previews. Upload textures on the thread that draws the panel.
- A short trigger press selects an entry, and a trigger drag scrolls the browser. When changing list or grid row heights, verify that the final row is reachable and that selection fires only on a short press.

## Verification

- Use the repository's `target` directory for local and verification builds.
- Wire changes through the existing module entry points. Use Rust ownership and `Drop` to manage native resource lifetimes. Document the type, alignment, and lifetime requirements of FFI pointer casts.
- Run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`. Use the Windows toolchain and commands in [build/README.md](build/README.md).
- Verify UI and VR changes with real videos and SteamVR. Check that thumbnails match their source clips and that both browser views can scroll and select entries in a large folder. Record which checks used a physical controller and which used automated input.
