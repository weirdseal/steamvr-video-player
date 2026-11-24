#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod files;
mod graphics;
mod i18n;
mod icons;
mod mpv;
mod settings;
mod settings_ui;
mod theme;
mod ui;
mod vr;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};

use i18n::Text;

const APP_NAME: &str = "SteamVR Video Player";

enum AppEvent {
    Media,
    Video,
    Repaint(Instant),
    FilePicked(Option<PathBuf>),
    NearbyVideos(u64, files::NearbyVideos),
    Vr(vr::Message),
}

fn executable_dir() -> Result<PathBuf> {
    std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .context(i18n::tr(Text::PlayerFolderUnavailable))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .init();
    let preferences = settings::Preferences::load();
    preferences.language.set_current();
    if let Err(error) = desktop::run(std::env::args_os().nth(1).map(PathBuf::from), preferences) {
        log::error!("{error:#}");
        let title = i18n::tr(Text::PlayerStartFailed);
        let message: Vec<u16> = format!("{title}\n\n{error:#}\0").encode_utf16().collect();
        let title: Vec<u16> = format!("{APP_NAME}\0").encode_utf16().collect();
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
