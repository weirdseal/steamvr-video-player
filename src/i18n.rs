use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/translations.rs"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Korean,
    English,
}

impl Language {
    pub const fn text(self, key: Text) -> &'static str {
        let catalog = match self {
            Self::Korean => KOREAN,
            Self::English => ENGLISH,
        };
        catalog[key as usize]
    }

    pub fn set_current(self) {
        ENGLISH_SELECTED.store(self == Self::English, Ordering::Relaxed);
    }
}

static ENGLISH_SELECTED: AtomicBool = AtomicBool::new(false);

pub fn tr(key: Text) -> &'static str {
    let language = if ENGLISH_SELECTED.load(Ordering::Relaxed) {
        Language::English
    } else {
        Language::Korean
    };
    language.text(key)
}
