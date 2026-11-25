use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use egui::scroll_area::ScrollSource;
use egui::{
    Align, Button, Color32, FontId, Label, Layout, Rect, Response, RichText, ScrollArea, Sense,
    TextWrapMode, Ui, Vec2, WidgetText, pos2, vec2,
};

use super::thumbnails::Thumbnails;
use crate::files::{self, Entry, Location, SortOrder};
use crate::i18n::{Language, Text, tr};
use crate::icons::{Icon, IconButton};
use crate::mpv::thumbnail;
use crate::settings::{BrowserView, Preferences};
use crate::settings_ui::sort_controls;
use crate::theme;

const GRID_COLUMNS: usize = 3;
const TILE_HEIGHT: f32 = 146.0;
const PREVIEW_HEIGHT: f32 = 102.0;
const ROW_HEIGHT: f32 = 44.0;

impl BrowserView {
    fn columns(self) -> usize {
        match self {
            Self::List => 1,
            Self::Grid => GRID_COLUMNS,
        }
    }

    fn row_height(self) -> f32 {
        match self {
            Self::List => ROW_HEIGHT,
            Self::Grid => TILE_HEIGHT,
        }
    }

    fn row_pitch(self) -> f32 {
        self.row_height() + self.row_spacing()
    }

    fn row_spacing(self) -> f32 {
        match self {
            Self::List => 3.0,
            Self::Grid => 10.0,
        }
    }
}

pub(super) enum BrowserAction {
    Browse(Location),
    View(BrowserView),
    Sort(SortOrder),
    Open(PathBuf),
    Close,
}

enum Listing {
    Loading(Receiver<io::Result<Vec<Entry>>>),
    Ready(Vec<Entry>),
    Failed(Option<io::Error>),
}

impl Listing {
    fn request(location: Location, sort: SortOrder) -> Self {
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("vr-file-browser".to_owned())
            .spawn(move || {
                let _ = sender.send(files::list(&location, sort));
            }) {
            Ok(_) => Self::Loading(receiver),
            Err(error) => Self::Failed(Some(error)),
        }
    }
}

pub(super) struct FileBrowser {
    location: Location,
    listing: Listing,
    view: BrowserView,
    sort: SortOrder,
    offset: f32,
    entries_rect: Option<Rect>,
    revision: u64,
}

impl FileBrowser {
    pub fn new(file: Option<&Path>, preferences: Preferences) -> Self {
        let location = file
            .and_then(Path::parent)
            .map_or(Location::Drives, |folder| {
                Location::Folder(folder.to_path_buf())
            });
        let listing = Listing::request(location.clone(), preferences.sort);
        Self {
            location,
            listing,
            view: preferences.browser_view,
            sort: preferences.sort,
            offset: 0.0,
            entries_rect: None,
            revision: 0,
        }
    }

    pub fn browse(&mut self, location: Location) {
        self.listing = Listing::request(location.clone(), self.sort);
        self.location = location;
        self.offset = 0.0;
        self.entries_rect = None;
        self.revision += 1;
    }

    pub fn scroll_by(&mut self, pixels: f32) {
        self.set_offset(self.offset + pixels);
    }

    pub fn entries_rect(&self) -> Option<Rect> {
        self.entries_rect
    }

    fn max_offset(&self) -> f32 {
        let (Listing::Ready(entries), Some(viewport)) = (&self.listing, self.entries_rect) else {
            return 0.0;
        };
        let rows = entries.len().div_ceil(self.view.columns());
        (rows as f32 * self.view.row_pitch() - self.view.row_spacing() - viewport.height()).max(0.0)
    }

    fn set_offset(&mut self, offset: f32) {
        let offset = offset.clamp(0.0, self.max_offset());
        if (offset - self.offset).abs() >= 0.001 {
            self.offset = offset;
            self.revision += 1;
        }
    }

    pub fn preferences(&mut self, preferences: Preferences) {
        if self.sort != preferences.sort {
            self.sort = preferences.sort;
            self.browse(self.location.clone());
        }
        self.set_view(preferences.browser_view);
    }

    fn set_view(&mut self, view: BrowserView) {
        if self.view != view {
            let first_row = (self.offset / self.view.row_pitch()).floor() as usize;
            let first_entry = first_row * self.view.columns();
            self.view = view;
            self.set_offset((first_entry / view.columns()) as f32 * view.row_pitch());
            self.entries_rect = None;
            self.revision += 1;
        }
    }

    pub fn poll(&mut self) -> Option<io::Error> {
        if let Listing::Loading(receiver) = &self.listing {
            self.listing = match receiver.try_recv() {
                Ok(Ok(entries)) => Listing::Ready(entries),
                Ok(Err(error)) => Listing::Failed(Some(error)),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    Listing::Failed(Some(io::Error::other(tr(Text::FolderListingInterrupted))))
                }
            };
            self.revision += 1;
        }
        match &mut self.listing {
            Listing::Failed(error) => error.take(),
            _ => None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn draw(
        &mut self,
        ui: &mut Ui,
        language: Language,
        thumbnails: &mut Thumbnails,
    ) -> Option<BrowserAction> {
        let mut action = None;
        let text = |key| language.text(key);
        let folder = match &self.location {
            Location::Drives => None,
            Location::Folder(path) => Some(path.as_path()),
        };
        let tool_size = [48.0, 44.0];
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 46.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                if ui
                    .add_enabled(
                        folder.is_some(),
                        IconButton::new(Icon::Up, text(Text::ParentFolder)).size(tool_size),
                    )
                    .clicked()
                {
                    let parent = folder.and_then(Path::parent).filter(|path| path.has_root());
                    action = Some(BrowserAction::Browse(
                        parent.map_or(Location::Drives, |path| {
                            Location::Folder(path.to_path_buf())
                        }),
                    ));
                }
                if ui
                    .add_enabled(
                        folder.is_some(),
                        IconButton::new(Icon::Drive, text(Text::Drives)).size(tool_size),
                    )
                    .clicked()
                {
                    action = Some(BrowserAction::Browse(Location::Drives));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Close, text(Text::Close)).size(tool_size))
                        .clicked()
                    {
                        action = Some(BrowserAction::Close);
                    }
                    for (view, icon, label) in [
                        (BrowserView::Grid, Icon::Grid, text(Text::GridView)),
                        (BrowserView::List, Icon::List, text(Text::ListView)),
                    ] {
                        if ui
                            .add(
                                IconButton::new(icon, label)
                                    .size(tool_size)
                                    .selected(self.view == view),
                            )
                            .clicked()
                        {
                            action = Some(BrowserAction::View(view));
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        draw_location(ui, folder, language);
                    });
                });
            },
        );

        let colors = &theme::COLORS;
        let mut offset = self.offset;
        self.entries_rect = None;
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(9, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(text(Text::Sort))
                            .size(20.0)
                            .color(colors.muted),
                    );
                    let mut sort = self.sort;
                    sort_controls(ui, &mut sort, language, 1.37);
                    if sort != self.sort {
                        action = Some(BrowserAction::Sort(sort));
                    }
                });
                let viewport_height = ui.available_height();
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), viewport_height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_min_height(viewport_height);
                        ui.spacing_mut().item_spacing.y = self.view.row_spacing();
                        match &self.listing {
                            Listing::Loading(_) => {
                                ui.label(RichText::new(text(Text::FolderReading)).size(25.0));
                            }
                            Listing::Failed(_) => {
                                if ui
                                    .add(button(text(Text::Retry), 143.0))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    action = Some(BrowserAction::Browse(self.location.clone()));
                                }
                            }
                            Listing::Ready(entries) if entries.is_empty() => {
                                ui.label(RichText::new(text(Text::NoFoldersOrVideos)).size(25.0));
                            }
                            Listing::Ready(entries) => {
                                let columns = self.view.columns();
                                let rows = entries.len().div_ceil(columns);
                                let output = ScrollArea::vertical()
                                    .id_salt("files")
                                    .max_height(viewport_height)
                                    .min_scrolled_height(viewport_height)
                                    .auto_shrink([false, false])
                                    .scroll_source(ScrollSource::SCROLL_BAR)
                                    .vertical_scroll_offset(self.offset)
                                    .show_rows(ui, self.view.row_height(), rows, |ui, rows| {
                                        let start = rows.start * columns;
                                        let end = entries.len().min(rows.end * columns);
                                        if let Some(selection) = draw_entries(
                                            ui,
                                            &entries[start..end],
                                            self.view,
                                            folder.is_some(),
                                            thumbnails,
                                        ) {
                                            action = Some(selection);
                                        }
                                    });
                                offset = output.state.offset.y;
                                self.entries_rect =
                                    Some(output.inner_rect.intersect(ui.clip_rect()));
                            }
                        }
                    },
                );
            });
        self.set_offset(offset);
        action
    }
}

fn draw_entries(
    ui: &mut Ui,
    entries: &[Entry],
    view: BrowserView,
    in_folder: bool,
    thumbnails: &mut Thumbnails,
) -> Option<BrowserAction> {
    let mut action = None;
    match view {
        BrowserView::List => {
            for entry in entries {
                if file_row(ui, entry, in_folder).clicked() {
                    action = Some(open_entry(entry));
                }
            }
        }
        BrowserView::Grid => {
            let width = (ui.available_width()
                - (GRID_COLUMNS - 1) as f32 * ui.spacing().item_spacing.x)
                / GRID_COLUMNS as f32;
            for row in entries.chunks(GRID_COLUMNS) {
                ui.horizontal(|ui| {
                    for entry in row {
                        if file_tile(ui, entry, width, in_folder, thumbnails).clicked() {
                            action = Some(open_entry(entry));
                        }
                    }
                });
            }
        }
    }
    action
}

fn open_entry(entry: &Entry) -> BrowserAction {
    if entry.directory {
        BrowserAction::Browse(Location::Folder(entry.path.clone()))
    } else {
        BrowserAction::Open(entry.path.clone())
    }
}

fn entry_region(ui: &mut Ui, entry: &Entry, size: Vec2) -> (Rect, Response) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &entry.name)
    });
    (
        rect,
        response.on_hover_cursor(egui::CursorIcon::PointingHand),
    )
}

fn entry_icon(entry: &Entry, in_folder: bool, colors: &theme::Colors) -> (Icon, Color32) {
    if entry.directory {
        (
            if in_folder { Icon::Folder } else { Icon::Drive },
            colors.accent,
        )
    } else {
        (Icon::Film, colors.muted)
    }
}

fn file_row(ui: &mut Ui, entry: &Entry, in_folder: bool) -> Response {
    let (rect, response) = entry_region(ui, entry, vec2(ui.available_width(), ROW_HEIGHT));
    if ui.is_rect_visible(rect) {
        let colors = &theme::COLORS;
        let painter = ui.painter_at(rect);
        if response.hovered() || response.has_focus() {
            painter.rect_filled(rect, 6, colors.hover);
        }
        let (icon, color) = entry_icon(entry, in_folder, colors);
        let center = pos2(rect.left() + 24.0, rect.center().y);
        icon.paint(&painter, center, 20.0, color);
        let galley = WidgetText::from(RichText::new(&entry.name).size(23.0)).into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            rect.width() - 66.0,
            egui::TextStyle::Button,
        );
        let position = pos2(rect.left() + 51.0, rect.center().y - galley.size().y * 0.5);
        painter.galley(position, galley, colors.text);
    }
    response
}

fn file_tile(
    ui: &mut Ui,
    entry: &Entry,
    width: f32,
    in_folder: bool,
    thumbnails: &mut Thumbnails,
) -> Response {
    let (rect, response) = entry_region(ui, entry, vec2(width, TILE_HEIGHT));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let colors = &theme::COLORS;
    let painter = ui.painter_at(rect);
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 9, colors.hover);
    }
    let preview = Rect::from_min_size(
        rect.min + vec2(7.0, 7.0),
        vec2(width - 14.0, PREVIEW_HEIGHT),
    );
    let texture = if entry.directory {
        None
    } else {
        thumbnails.texture(entry)
    };
    if let Some(texture) = texture {
        let source = vec2(thumbnail::SIZE[0] as f32, thumbnail::SIZE[1] as f32);
        let scale = (preview.width() / source.x).max(preview.height() / source.y);
        let uv = Rect::from_center_size(pos2(0.5, 0.5), preview.size() / (source * scale));
        egui::Image::new((texture, preview.size()))
            .uv(uv)
            .corner_radius(6)
            .paint_at(ui, preview);
    } else {
        let (icon, color) = entry_icon(entry, in_folder, colors);
        icon.paint(&painter, preview.center(), 40.0, color);
    }
    let galley = WidgetText::from(RichText::new(&entry.name).size(22.0)).into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        width - 24.0,
        egui::TextStyle::Button,
    );
    let position = pos2(rect.left() + 12.0, rect.bottom() - 8.0 - galley.size().y);
    painter.galley(position, galley, colors.text);
    response
}

fn draw_location(ui: &mut Ui, folder: Option<&Path>, language: Language) {
    let colors = &theme::COLORS;
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(13, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let font = FontId::proportional(21.0);
            let full_path =
                folder.map_or(language.text(Text::ThisPc).into(), Path::to_string_lossy);
            let width = ui
                .painter()
                .layout_no_wrap(full_path.to_string(), font.clone(), colors.text)
                .size()
                .x;
            let location = if width > ui.available_width()
                && let Some(name) = folder.and_then(Path::file_name)
            {
                format!("…\\{}", name.to_string_lossy()).into()
            } else {
                full_path.clone()
            };
            ui.add(
                Label::new(
                    RichText::new(location.as_ref())
                        .font(font)
                        .color(colors.text),
                )
                .truncate(),
            )
            .on_hover_text(full_path.as_ref());
        });
}

fn button(label: &str, width: f32) -> Button<'_> {
    Button::new(RichText::new(label).size(24.0)).min_size(Vec2::new(width, 42.0))
}
