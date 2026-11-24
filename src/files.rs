use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
use windows_sys::Win32::UI::Shell::StrCmpLogicalW;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "3gp", "asf", "avi", "divx", "f4v", "flv", "m2t", "m2ts", "m4v", "mkv", "mov", "mp4", "mpe",
    "mpeg", "mpg", "mts", "mxf", "ogv", "rm", "rmvb", "ts", "vob", "webm", "wmv",
];

#[derive(Clone, PartialEq, Eq)]
pub enum Location {
    Drives,
    Folder(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Name,
    Modified,
    Size,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SortOrder {
    pub field: SortField,
    pub descending: bool,
}

pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub directory: bool,
    name_key: FileName,
    modified: Option<SystemTime>,
    size: Option<u64>,
}

impl Entry {
    fn new(name: String, path: PathBuf, directory: bool, metadata: Option<&fs::Metadata>) -> Self {
        Self {
            name_key: FileName::new(&name),
            name,
            path,
            directory,
            modified: metadata.and_then(|metadata| metadata.modified().ok()),
            size: metadata.filter(|_| !directory).map(fs::Metadata::len),
        }
    }

    pub fn file_stamp(&self) -> (Option<SystemTime>, Option<u64>) {
        (self.modified, self.size)
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct NearbyVideos {
    pub previous: Option<Arc<Path>>,
    pub next: Option<Arc<Path>>,
}

pub fn list(location: &Location, sort: SortOrder) -> io::Result<Vec<Entry>> {
    let mut entries = match location {
        Location::Drives => drives(),
        Location::Folder(path) => folder(path),
    }?;
    sort_entries(&mut entries, sort);
    Ok(entries)
}

fn drives() -> io::Result<Vec<Entry>> {
    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((0..26)
        .filter(|&index| mask & (1 << index) != 0)
        .map(|index| {
            let name = format!("{}:\\", char::from(b'A' + index));
            let path = PathBuf::from(&name);
            Entry::new(name, path, true, None)
        })
        .collect())
}

fn folder(path: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(path)? {
        let item = item?;
        let path = item.path();
        let kind = item.file_type()?;
        let metadata = if kind.is_symlink() {
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            Some(metadata)
        } else {
            item.metadata().ok()
        };
        let directory = metadata
            .as_ref()
            .map_or(kind.is_dir(), fs::Metadata::is_dir);
        let regular_file = metadata
            .as_ref()
            .map_or(kind.is_file(), fs::Metadata::is_file);
        let video = regular_file
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    VIDEO_EXTENSIONS
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(extension))
                });
        if directory || video {
            entries.push(Entry::new(
                item.file_name().to_string_lossy().into_owned(),
                path,
                directory,
                metadata.as_ref(),
            ));
        }
    }
    Ok(entries)
}

fn sort_entries(entries: &mut [Entry], sort: SortOrder) {
    let direction = |ordering: Ordering| {
        if sort.descending {
            ordering.reverse()
        } else {
            ordering
        }
    };
    entries.sort_unstable_by(|left, right| {
        right.directory.cmp(&left.directory).then_with(|| {
            let name = || left.name_key.cmp(&right.name_key);
            match sort.field {
                SortField::Name => direction(name()),
                SortField::Modified => {
                    compare_known(left.modified, right.modified, direction).then_with(name)
                }
                SortField::Size => compare_known(left.size, right.size, direction).then_with(name),
            }
        })
    });
}

fn compare_known<T: Ord>(
    left: Option<T>,
    right: Option<T>,
    direction: impl FnOnce(Ordering) -> Ordering,
) -> Ordering {
    // Missing metadata stays last in either direction.
    right
        .is_some()
        .cmp(&left.is_some())
        .then_with(|| direction(left.cmp(&right)))
}

pub fn nearby(file: &Path, sort: SortOrder) -> io::Result<NearbyVideos> {
    let (Some(parent), Some(name)) = (file.parent(), file.file_name()) else {
        return Ok(NearbyVideos::default());
    };
    let mut files = folder(parent)?;
    files.retain(|entry| !entry.directory);
    let name = name.to_string_lossy();
    let current_name = files
        .iter()
        .find(|entry| entry.name == name)
        .or_else(|| {
            let name = name.to_lowercase();
            files.iter().find(|entry| entry.name.to_lowercase() == name)
        })
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| {
            // The Windows picker also accepts playable files with an unlisted extension.
            let metadata = fs::metadata(file).ok();
            files.push(Entry::new(
                name.to_string(),
                file.to_path_buf(),
                false,
                metadata.as_ref(),
            ));
            name.into_owned()
        });
    sort_entries(&mut files, sort);
    let Some(current) = files.iter().position(|entry| entry.name == current_name) else {
        return Ok(NearbyVideos::default());
    };
    Ok(NearbyVideos {
        previous: current
            .checked_sub(1)
            .map(|index| Arc::from(files[index].path.as_path())),
        next: files
            .get(current + 1)
            .map(|entry| Arc::from(entry.path.as_path())),
    })
}

#[derive(PartialEq, Eq)]
struct FileName(Vec<u16>);

impl FileName {
    fn new(name: &str) -> Self {
        Self(name.encode_utf16().chain([0]).collect())
    }
}

impl Ord for FileName {
    fn cmp(&self, other: &Self) -> Ordering {
        let logical = unsafe { StrCmpLogicalW(self.0.as_ptr(), other.0.as_ptr()) };
        logical.cmp(&0).then_with(|| self.0.cmp(&other.0))
    }
}

impl PartialOrd for FileName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn each_sort_keeps_folders_first_and_breaks_ties_by_natural_name() {
        let video = |name: &str, size, time: Option<u64>| {
            let mut entry = Entry::new(name.to_owned(), PathBuf::from(name), false, None);
            entry.size = size;
            entry.modified = time.map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds));
            entry
        };
        let mut entries = [
            video("10.mp4", Some(10), Some(20_u64)),
            video("2.mp4", Some(10), Some(10_u64)),
            video("1.mp4", None, None),
            Entry::new("Folder".to_owned(), PathBuf::from("Folder"), true, None),
        ];
        let names = |entries: &[Entry]| {
            entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>()
        };
        sort_entries(&mut entries, SortOrder::default());
        assert_eq!(names(&entries), ["Folder", "1.mp4", "2.mp4", "10.mp4"]);
        sort_entries(
            &mut entries,
            SortOrder {
                field: SortField::Size,
                descending: true,
            },
        );
        assert_eq!(names(&entries), ["Folder", "2.mp4", "10.mp4", "1.mp4"]);
        sort_entries(
            &mut entries,
            SortOrder {
                field: SortField::Modified,
                descending: true,
            },
        );
        assert_eq!(names(&entries), ["Folder", "10.mp4", "2.mp4", "1.mp4"]);
    }
}
