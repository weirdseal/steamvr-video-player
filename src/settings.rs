use std::fs;
use std::io;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{APP_NAME, files::SortOrder, graphics::VrSettings, i18n::Language};

pub const SEEK_INTERVAL: RangeInclusive<u8> = 1..=60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserView {
    #[default]
    List,
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub language: Language,
    pub sort: SortOrder,
    pub browser_view: BrowserView,
    pub initial_volume: u8,
    pub volume_step: u8,
    pub pc_seek: u8,
    pub vr_seek: u8,
    pub play_on_open: bool,
    pub panel_timeout: u8,
    pub vr: VrSettings,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: Language::Korean,
            sort: SortOrder::default(),
            browser_view: BrowserView::List,
            initial_volume: 75,
            volume_step: 5,
            pc_seek: 5,
            vr_seek: 10,
            play_on_open: true,
            panel_timeout: 8,
            vr: VrSettings::default(),
        }
    }
}

impl Preferences {
    pub fn load() -> Self {
        let result = (|| -> Result<Self> {
            let path = settings_path()?;
            match fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid settings file"),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
                Err(error) => Err(error).context("Could not read settings"),
            }
        })();
        let mut preferences = result.unwrap_or_else(|error| {
            log::warn!("{error:#}");
            Self::default()
        });
        preferences.normalize();
        preferences
    }

    pub fn save(self) -> Result<()> {
        let path = settings_path()?;
        let directory = path.parent().context("Settings directory is unavailable")?;
        fs::create_dir_all(directory)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let file = fs::File::create(&temporary)?;
            serde_json::to_writer_pretty(&file, &self)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    fn normalize(&mut self) {
        self.initial_volume = self.initial_volume.min(100);
        self.volume_step = self.volume_step.clamp(1, 25);
        self.pc_seek = self
            .pc_seek
            .clamp(*SEEK_INTERVAL.start(), *SEEK_INTERVAL.end());
        self.vr_seek = self
            .vr_seek
            .clamp(*SEEK_INTERVAL.start(), *SEEK_INTERVAL.end());
        self.panel_timeout = self.panel_timeout.clamp(3, 30);
        self.vr.screen_width = self.vr.screen_width.clamp(0.5, 15.0);
        self.vr.screen_distance = self.vr.screen_distance.clamp(0.5, 15.0);
    }

    pub fn apply_edit(&mut self, before: Self, edited: Self) {
        // A VR edit changes only the fields the user touched, even if PC settings changed meanwhile.
        macro_rules! apply {
            ($($($field:ident).+),+ $(,)?) => {$(
                if before.$($field).+ != edited.$($field).+ {
                    self.$($field).+ = edited.$($field).+;
                }
            )+};
        }
        apply!(
            language,
            sort.field,
            sort.descending,
            browser_view,
            initial_volume,
            volume_step,
            pc_seek,
            vr_seek,
            play_on_open,
            panel_timeout,
            vr.projection,
            vr.stereo,
            vr.half_resolution,
            vr.screen_width,
            vr.screen_distance,
        );
        self.normalize();
    }
}

fn settings_path() -> Result<PathBuf> {
    let app_data = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?;
    Ok(PathBuf::from(app_data).join(APP_NAME).join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::SortField;

    #[test]
    fn a_settings_edit_preserves_other_recent_changes() {
        let before = Preferences::default();
        let mut edited = before;
        edited.sort.field = SortField::Size;
        let mut current = Preferences {
            language: Language::English,
            ..before
        };
        current.apply_edit(before, edited);
        assert_eq!(current.language, Language::English);
        assert_eq!(current.sort.field, SortField::Size);
    }

    #[test]
    fn missing_settings_use_defaults_and_values_are_bounded() {
        let mut settings: Preferences =
            serde_json::from_str(r#"{"pc_seek":0,"vr":{"screen_width":17.0}}"#).unwrap();
        settings.normalize();
        assert_eq!(settings.pc_seek, 1);
        assert_eq!(settings.vr.screen_width, 15.0);
        assert_eq!(settings.vr_seek, Preferences::default().vr_seek);
        assert_eq!(
            serde_json::from_slice::<Preferences>(&serde_json::to_vec(&settings).unwrap()).unwrap(),
            settings
        );
    }
}
