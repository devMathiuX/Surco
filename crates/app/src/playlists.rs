//! Playlists propias y cambios a las de la biblioteca: crear, renombrar,
//! agregar y quitar canciones, cambiar el orden (arrastrando en la barra
//! lateral), borrar lo descargado de una playlist y las canciones
//! recomendadas que aparecen al final de cada una.

use std::collections::HashMap;

use iced::widget::{
    button, column, container, mouse_area, opaque, operation, row, scrollable, stack, text, Column,
    Id, Space,
};
use iced::{Alignment, Element, Length, Size, Task};
use surco_core::{format_duration, Playlist, Track};
use surco_provider::SearchResult;
use surco_resolver::cache_key;

use crate::playback::TrackRes;
use crate::radio::{track_from, RADIO_ID};
use crate::{skeleton, theme, Message, Notice, Screen, Surco, RUNTIME};

pub const RENAME_INPUT: &str = "rename-input";
/// Origen de las playlists creadas en Surco.
pub const OWN_SOURCE: &str = "propia";
/// Recomendadas que se muestran al final de una playlist.
const RECS_SHOWN: usize = 10;
/// Videos que se piden al Mix de cada semilla.
const RECS_MIX: usize = 25;

/// Recomendadas para una playlist (por ID).
#[derive(Debug, Clone)]
pub enum Recs {
    Loading,
    Ready(Vec<SearchResult>),
    Failed,
}

/// Menú de una canción ("+"): dónde agregarla y, si se abrió desde una
/// playlist, quitarla de ahí.
#[derive(Debug, Clone)]
pub struct SongMenu {
    pub track: Track,
    /// Playlist y posición de donde se abrió.
    pub from: Option<(usize, usize)>,
}

/// Opción elegida en el menú de una canción.
#[derive(Debug, Clone, Copy)]
pub enum AddTarget {
    /// Al final de la cola.
    Queue,
    /// Después de la que suena.
    Next,
    Playlist(usize),
    /// Una playlist nueva con esta canción.
    NewPlaylist,
    /// Quitarla de la playlist de donde se abrió el menú.
    Remove,
}

/// Bytes y canciones descargadas por playlist (ID).
pub type Downloads = HashMap<String, (usize, u64)>;

/// Nueva posición del elemento que estaba en `i` después de mover `from` a `to`.
pub fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// ID nuevo para una playlist propia.
fn new_id() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("user:{ms}")
}

/// "Mi playlist", "Mi playlist 2"… (el primero libre).
pub fn free_name(existing: &[&str]) -> String {
    let base = "Mi playlist";
    (1..)
        .map(|n| {
            if n == 1 {
                base.to_owned()
            } else {
                format!("{base} {n}")
            }
        })
        .find(|name| !existing.contains(&name.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

impl Surco {
    /// Crea una playlist vacía (o con una canción), la abre y deja el nombre
    /// listo para cambiarlo.
    pub(crate) fn new_playlist(&mut self, first: Option<Track>) -> Task<Message> {
        let names: Vec<&str> = self
            .playlists
            .iter()
            .map(|(p, _)| p.name.as_str())
            .collect();
        let playlist = Playlist {
            id: new_id(),
            name: free_name(&names),
            tracks: first.into_iter().collect(),
        };
        self.persist_playlist(&playlist, OWN_SOURCE);
        self.playlists.push((playlist, OWN_SOURCE.into()));
        let index = self.playlists.len() - 1;
        self.open_playlist(index);
        let opened = self.on_playlist_opened();
        Task::batch([opened, self.start_rename(index)])
    }

    /// Abre una playlist en el panel principal.
    pub(crate) fn open_playlist(&mut self, index: usize) {
        self.selected = Some(index);
        self.screen = Screen::Library;
        self.filter = None;
        self.list_view.0 = 0.0;
        self.persist_selected();
    }

    pub(crate) fn start_rename(&mut self, index: usize) -> Task<Message> {
        let Some((p, _)) = self.playlists.get(index) else {
            return Task::none();
        };
        self.renaming = Some((index, p.name.clone()));
        Task::batch([
            operation::focus(Id::new(RENAME_INPUT)),
            operation::select_all(Id::new(RENAME_INPUT)),
        ])
    }

    pub(crate) fn commit_rename(&mut self) {
        let Some((index, name)) = self.renaming.take() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() || index >= self.playlists.len() {
            return;
        }
        // La radio se reemplaza con cada radio nueva: al ponerle nombre pasa a
        // ser una playlist más.
        if self.playlists[index].0.id == RADIO_ID {
            let id = new_id();
            if let Some(store) = &mut self.store {
                if let Err(e) = store.change_playlist_id(RADIO_ID, &id) {
                    tracing::warn!(error = %e, "guardar la radio como playlist");
                }
            }
            if let Some(r) = self.recs.remove(RADIO_ID) {
                self.recs.insert(id.clone(), r);
            }
            self.playlists[index].0.id = id;
        }
        self.playlists[index].0.name = name.clone();
        if let Some(store) = &self.store {
            if let Err(e) = store.rename_playlist(&self.playlists[index].0.id, &name) {
                tracing::warn!(error = %e, "renombrar playlist");
            }
        }
        self.persist_selected();
    }

    /// Opción elegida en el menú de una canción.
    pub(crate) fn song_menu_pick(&mut self, target: AddTarget) -> Task<Message> {
        let Some(menu) = self.song_menu.take() else {
            return Task::none();
        };
        match target {
            AddTarget::Queue => self.enqueue(menu.track, false),
            AddTarget::Next => self.enqueue(menu.track, true),
            AddTarget::Playlist(p) => {
                self.add_to_playlist(p, menu.track);
                Task::none()
            }
            AddTarget::NewPlaylist => self.new_playlist(Some(menu.track)),
            AddTarget::Remove => {
                if let Some((p, i)) = menu.from {
                    self.remove_from_playlist(p, i);
                }
                Task::none()
            }
        }
    }

    /// Agrega una canción al final de una playlist.
    pub(crate) fn add_to_playlist(&mut self, p: usize, track: Track) {
        let Some((playlist, source)) = self.playlists.get_mut(p) else {
            return;
        };
        let name = playlist.name.clone();
        let old_len = playlist.tracks.len();
        playlist.tracks.push(track.clone());
        let (playlist, source) = (playlist.clone(), source.clone());
        // Que el video elegido quede guardado con la canción.
        if let Some(TrackRes::Done(m)) = self.resolutions.get(&cache_key(&track)).cloned() {
            self.persist_match(&cache_key(&track), &m);
        }
        self.persist_playlist(&playlist, &source);
        // Si la playlist está sonando, la canción nueva entra en la cola.
        if let Some(a) = self.active.as_mut().filter(|a| a.origin == Some(p)) {
            if a.tracks.len() == old_len {
                a.tracks.push(track);
                a.queue.push(old_len);
            } else {
                a.origin = None;
            }
        }
        if let Some(Recs::Ready(list)) = self.recs.get_mut(&playlist.id) {
            let video = self.resolutions.get(&cache_key(&playlist.tracks[old_len]));
            if let Some(TrackRes::Done(m)) = video {
                list.retain(|r| r.id != m.youtube_id);
            }
        }
        self.notice = Some(Notice {
            error: false,
            text: trf!("Agregada a «{name}».", "Added to “{name}”."),
        });
        self.update_preload();
        self.refresh_downloads();
    }

    /// Quita la canción `i` de la playlist `p`.
    pub(crate) fn remove_from_playlist(&mut self, p: usize, i: usize) {
        let Some((playlist, source)) = self.playlists.get_mut(p) else {
            return;
        };
        if i >= playlist.tracks.len() {
            return;
        }
        playlist.tracks.remove(i);
        let (playlist, source) = (playlist.clone(), source.clone());
        self.persist_playlist(&playlist, &source);
        // Los índices de la cola ya no coinciden: sigue sonando, suelta.
        if let Some(a) = self.active.as_mut().filter(|a| a.origin == Some(p)) {
            a.origin = None;
        }
        if self.choosing.is_some_and(|(cp, _)| cp == p) {
            self.choosing = None;
        }
        self.refresh_downloads();
    }

    /// Mueve una playlist de la barra lateral de `from` a `to`.
    pub(crate) fn move_playlist(&mut self, from: usize, to: usize) {
        let len = self.playlists.len();
        if from == to || from >= len || to >= len {
            return;
        }
        let item = self.playlists.remove(from);
        self.playlists.insert(to, item);
        let remap = |i: usize| moved_index(i, from, to);
        self.selected = self.selected.map(remap);
        if let Some(a) = &mut self.active {
            a.origin = a.origin.map(remap);
        }
        self.choosing = self.choosing.map(|(p, i)| (remap(p), i));
        self.renaming = self.renaming.take().map(|(p, n)| (remap(p), n));
        if let Some(menu) = &mut self.song_menu {
            menu.from = menu.from.map(|(p, i)| (remap(p), i));
        }
        let ids: Vec<&str> = self.playlists.iter().map(|(p, _)| p.id.as_str()).collect();
        if let Some(store) = &mut self.store {
            if let Err(e) = store.set_playlist_order(&ids) {
                tracing::warn!(error = %e, "guardar el orden de las playlists");
            }
        }
    }

    // ── Descargas ──────────────────────────────────────────────────────────

    /// Recalcula cuánto hay descargado de cada playlist.
    pub(crate) fn refresh_downloads(&mut self) {
        let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) else {
            self.downloads.clear();
            return;
        };
        let mut out = Downloads::new();
        for (p, _) in &self.playlists {
            let mut seen = std::collections::HashSet::new();
            let (mut files, mut bytes) = (0, 0);
            for t in &p.tracks {
                let Some(v) = self.video_of_track(t) else {
                    continue;
                };
                if !seen.insert(v) {
                    continue;
                }
                if let Some(len) = cache.size_of(v) {
                    files += 1;
                    bytes += len;
                }
            }
            if files > 0 {
                out.insert(p.id.clone(), (files, bytes));
            }
        }
        self.downloads = out;
        self.update_pins();
    }

    // ── Recomendadas ───────────────────────────────────────────────────────

    /// Pide recomendadas para la playlist abierta, si todavía no hay.
    pub(crate) fn maybe_load_recs(&mut self) -> Task<Message> {
        match self.selected {
            Some(p) if self.screen == Screen::Library => self.load_recs(p, false),
            _ => Task::none(),
        }
    }

    /// Recomendadas para la playlist `p`: parecidas (Mix de YouTube) a dos
    /// canciones suyas ya encontradas. `again` pide otras.
    pub(crate) fn load_recs(&mut self, p: usize, again: bool) -> Task<Message> {
        let Some((playlist, _)) = self.playlists.get(p) else {
            return Task::none();
        };
        let id = playlist.id.clone();
        let current = self.recs.get(&id);
        if matches!(current, Some(Recs::Loading)) || (current.is_some() && !again) {
            return Task::none();
        }
        let videos: Vec<String> = playlist
            .tracks
            .iter()
            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
            .collect();
        if videos.is_empty() {
            return Task::none();
        }
        let (Some(provider), Some(rt)) = (self.provider.clone(), RUNTIME.get()) else {
            return Task::none();
        };
        // Semillas: una al azar, la última y la primera ("Otras" cambia la
        // de azar). Varias, porque el Mix de algún video puede fallar.
        let n = videos.len();
        let mut seeds = vec![videos[(crate::playback::seed() as usize) % n].clone()];
        for v in [&videos[n - 1], &videos[0]] {
            if !seeds.contains(v) && (!again || seeds.len() < 2) {
                seeds.push(v.clone());
            }
        }
        // Si ningún Mix anda (videos que YouTube no muestra en esa región),
        // se buscan canciones de los artistas de la playlist.
        let artist = playlist
            .tracks
            .get((crate::playback::seed() as usize / 7) % playlist.tracks.len())
            .map(|t| t.main_artist().to_owned());
        self.recs.insert(id.clone(), Recs::Loading);
        let job = rt.spawn(async move {
            let y = provider.ytdlp();
            let mut all = Vec::new();
            let mut last_err = None;
            let lists = iced::futures::future::join_all(seeds.iter().map(|s| y.mix(s, RECS_MIX)));
            for r in lists.await {
                match r {
                    Ok(l) => all.extend(l),
                    Err(e) => last_err = Some(e.to_string()),
                }
            }
            if all.is_empty() {
                if let Some(a) = artist.filter(|a| !a.is_empty()) {
                    match y.search(&a, RECS_MIX).await {
                        Ok(l) => all.extend(l),
                        Err(e) => last_err = Some(e.to_string()),
                    }
                }
            }
            match (all.is_empty(), last_err) {
                (true, Some(e)) => Err(e),
                _ => Ok(all),
            }
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            move |r| Message::RecsLoaded(id.clone(), r),
        )
    }

    pub(crate) fn on_recs(
        &mut self,
        id: String,
        result: Result<Vec<SearchResult>, String>,
    ) -> Task<Message> {
        let list = match result {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "sin recomendadas");
                self.recs.insert(id, Recs::Failed);
                return Task::none();
            }
        };
        let Some((playlist, _)) = self.playlists.iter().find(|(p, _)| p.id == id) else {
            return Task::none();
        };
        let mut known: std::collections::HashSet<String> = playlist
            .tracks
            .iter()
            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
            .collect();
        // Tampoco otra versión con el mismo nombre de una que ya está.
        let title_key = |t: &Track| format!("{}|{}", t.main_artist(), t.title).to_lowercase();
        let mut titles: std::collections::HashSet<String> =
            playlist.tracks.iter().map(title_key).collect();
        let mut out: Vec<SearchResult> = list
            .into_iter()
            .filter(|r| {
                r.duration
                    .is_none_or(|d| (60..=15 * 60).contains(&d.as_secs()))
            })
            .filter(|r| known.insert(r.id.clone()))
            .filter(|r| titles.insert(title_key(&track_from(r))))
            .collect();
        // Mezcla las de las dos semillas para que no salgan todas del mismo artista.
        let half = out.len().div_ceil(2);
        let (a, b) = out.split_at(half);
        let mut mixed = Vec::with_capacity(out.len());
        for i in 0..half {
            mixed.extend(a.get(i).cloned());
            mixed.extend(b.get(i).cloned());
        }
        out = mixed;
        out.truncate(RECS_SHOWN);
        let covers: Vec<String> = out.iter().map(|r| r.id.clone()).collect();
        self.recs.insert(id, Recs::Ready(out));
        Task::batch(
            covers
                .iter()
                .map(|v| self.queue_cover(v))
                .collect::<Vec<_>>(),
        )
    }

    /// Agregar la recomendada `i` a la playlist `p`.
    pub(crate) fn add_rec(&mut self, p: usize, i: usize) {
        let Some(r) = self.rec(p, i) else {
            return;
        };
        let track = self.adopt(&r);
        self.add_to_playlist(p, track);
    }

    /// Escuchar la recomendada `i` de la playlist `p` (sigue con parecidas).
    pub(crate) fn play_rec(&mut self, p: usize, i: usize) -> Task<Message> {
        let Some(r) = self.rec(p, i) else {
            return Task::none();
        };
        let track = self.adopt(&r);
        self.play_list(vec![track], 0, true)
    }

    fn rec(&self, p: usize, i: usize) -> Option<SearchResult> {
        let (playlist, _) = self.playlists.get(p)?;
        match self.recs.get(&playlist.id)? {
            Recs::Ready(list) => list.get(i).cloned(),
            _ => None,
        }
    }

    // ── Vistas ─────────────────────────────────────────────────────────────

    /// Sección "Recomendadas" al final de una playlist.
    pub(crate) fn view_recs(&self, p: usize, playlist: &Playlist) -> Element<'_, Message> {
        let title = column![
            text(tr!("Recomendadas", "Recommended")).size(theme::SIZE_M),
            text(tr!(
                "Parecidas a las canciones de esta playlist. Agregalas para hacerla más grande.",
                "Similar to the songs in this playlist. Add them to make it bigger."
            ))
            .size(theme::SIZE_S)
            .style(theme::secondary_text),
        ]
        .spacing(2)
        .width(Length::Fill);
        let more = button(text(tr!("Otras", "Others")).size(theme::SIZE_S))
            .on_press(Message::RecsRefresh(p))
            .padding([4, 10])
            .style(theme::ghost_button);
        let body: Element<'_, Message> = match self.recs.get(&playlist.id) {
            None if playlist.tracks.is_empty() => text(
                tr!("Agregá canciones con el + del buscador y acá van a aparecer recomendaciones.", "Add songs with the + in search and recommendations will show up here."),
            )
            .size(theme::SIZE_S)
            .style(theme::secondary_text)
            .into(),
            None if self.background.is_running() => skeleton::song_rows(3, self.pulse),
            None => text(
                tr!("Las recomendaciones aparecen cuando Surco encuentra en YouTube alguna canción de esta playlist (por ejemplo, al escucharla).", "Recommendations show up once Surco finds a song from this playlist on YouTube (for example, when you play it)."),
            )
            .size(theme::SIZE_S)
            .style(theme::secondary_text)
            .into(),
            Some(Recs::Loading) => skeleton::song_rows(3, self.pulse),
            Some(Recs::Failed) => text(tr!("No pude traer recomendaciones. Probá con «Otras».", "Couldn't get recommendations. Try “Others”."))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into(),
            Some(Recs::Ready(list)) if list.is_empty() => {
                text(tr!("No encontré canciones nuevas para recomendar. Probá con «Otras».", "Found no new songs to recommend. Try “Others”."))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into()
            }
            Some(Recs::Ready(list)) => {
                Column::with_children(list.iter().enumerate().map(|(i, r)| self.rec_row(p, i, r)))
                    .spacing(2)
                    .into()
            }
        };
        column![
            row![title, more].align_y(Alignment::Center).spacing(12),
            body
        ]
        .spacing(12)
        .padding(iced::Padding::ZERO.top(28))
        .into()
    }

    fn rec_row<'a>(&'a self, p: usize, i: usize, r: &'a SearchResult) -> Element<'a, Message> {
        let t = track_from(r);
        let cover: Element<'_, Message> = match self.covers.get(&r.id) {
            Some(h) => iced::widget::image(h.clone())
                .width(36)
                .height(36)
                .border_radius(4.0)
                .into(),
            None => container(Space::new())
                .width(36)
                .height(36)
                .style(theme::cover_placeholder)
                .into(),
        };
        let duration = r
            .duration
            .map(|d| format_duration(d.as_millis() as u64))
            .unwrap_or_default();
        button(
            row![
                Space::new().width(40),
                cover,
                column![
                    text(t.title)
                        .size(theme::SIZE_M)
                        .wrapping(text::Wrapping::None),
                    text(t.artists.join(", "))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text)
                        .wrapping(text::Wrapping::None),
                ]
                .spacing(2)
                .width(Length::Fill)
                .clip(true),
                button(text(tr!("Agregar", "Add")).size(theme::SIZE_S))
                    .on_press(Message::RecAdd(p, i))
                    .padding([4, 10])
                    .style(theme::ghost_button),
                text(duration)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(Message::RecPlay(p, i))
        .padding([6, 8])
        .width(Length::Fill)
        .style(theme::list_button(false))
        .into()
    }

    /// Diálogo del menú de una canción, sobre toda la ventana.
    /// Menú de clic derecho de una playlist de la barra lateral.
    pub(crate) fn view_playlist_menu(&self, index: usize) -> Element<'_, Message> {
        let Some((p, _)) = self.playlists.get(index) else {
            return Space::new().into();
        };
        let option = |label: String, msg: Message, danger: bool| {
            button(text(label).size(theme::SIZE_S).style(if danger {
                theme::error_text
            } else {
                theme::plain_text
            }))
            .on_press(msg)
            .width(Length::Fill)
            .padding([8, 10])
            .style(theme::list_button(false))
        };
        let mut options = column![option(
            tr!("Renombrar", "Rename").into(),
            Message::RenameFromMenu(index),
            false
        )]
        .spacing(2);
        if let Some((files, bytes)) = self.downloads.get(&p.id) {
            options = options.push(option(
                trf!(
                    "Borrar descargas ({}, {})",
                    "Delete downloads ({}, {})",
                    crate::view::songs(*files),
                    crate::view::format_bytes(*bytes)
                ),
                Message::DeleteDownloads(index),
                false,
            ));
        }
        options = options.push(option(
            tr!("Quitar playlist", "Remove playlist").into(),
            Message::RemovePlaylist(index),
            true,
        ));
        let card = container(
            column![
                container(
                    text(&p.name)
                        .size(theme::SIZE_M)
                        .wrapping(text::Wrapping::None)
                )
                .padding([0, 10])
                .clip(true),
                options,
            ]
            .spacing(10),
        )
        .padding(14)
        .width(PLAYLIST_MENU_WIDTH)
        .style(theme::dialog);
        let rows = 2 + usize::from(self.downloads.contains_key(&p.id));
        let size = Size::new(PLAYLIST_MENU_WIDTH, 64.0 + rows as f32 * 37.0);
        self.anchored(card, size, Message::PlaylistMenu(None))
    }

    pub(crate) fn view_song_menu<'a>(&'a self, menu: &'a SongMenu) -> Element<'a, Message> {
        let option = |label: String, target: AddTarget, danger: bool| {
            button(text(label).size(theme::SIZE_S).style(if danger {
                theme::error_text
            } else {
                theme::plain_text
            }))
            .on_press(Message::SongMenuPick(target))
            .width(Length::Fill)
            .padding([8, 10])
            .style(theme::list_button(false))
        };
        let mut playlists = Column::new().spacing(2);
        let mut others = 0;
        for (i, (p, _)) in self.playlists.iter().enumerate() {
            if menu.from.is_some_and(|(fp, _)| fp == i) {
                continue;
            }
            others += 1;
            playlists = playlists.push(option(p.name.clone(), AddTarget::Playlist(i), false));
        }
        let mut col = column![
            column![
                text(tr!("Agregar a…", "Add to…")).size(theme::SIZE_M),
                text(format!(
                    "{} · {}",
                    menu.track.title,
                    menu.track.artists.join(", ")
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .wrapping(text::Wrapping::None),
            ]
            .spacing(2)
            .padding([0, 10]),
            column![
                option(
                    tr!("La cola (al final)", "The queue (at the end)").into(),
                    AddTarget::Queue,
                    false
                ),
                option(
                    tr!("La cola (que suene después)", "The queue (play next)").into(),
                    AddTarget::Next,
                    false
                ),
                option(
                    tr!("+ Nueva playlist", "+ New playlist").into(),
                    AddTarget::NewPlaylist,
                    false
                ),
            ]
            .spacing(2),
        ]
        .spacing(10);
        if others > 0 {
            col = col.push(
                container(
                    text(tr!("Tus playlists", "Your playlists"))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                )
                .padding([0, 10]),
            );
            col = col.push(scrollable(playlists).height(Length::Shrink));
        }
        if let Some((p, _)) = menu
            .from
            .and_then(|(p, _)| self.playlists.get(p).map(|x| (p, x)))
        {
            let name = &self.playlists[p].0.name;
            col = col.push(option(
                trf!("Quitar de «{name}»", "Remove from “{name}”"),
                AddTarget::Remove,
                true,
            ));
        }
        // Alto aproximado, para saber si entra abajo del mouse.
        let rows = 3 + others + usize::from(menu.from.is_some());
        let extra = if others > 0 { 30.0 } else { 0.0 };
        let height = (70.0 + rows as f32 * 37.0 + extra).min(MENU_MAX);
        let card = container(col.clip(true))
            .padding(14)
            .width(SONG_MENU_WIDTH)
            .max_height(MENU_MAX)
            .style(theme::dialog);
        self.anchored(
            card,
            Size::new(SONG_MENU_WIDTH, height),
            Message::SongMenu(None),
        )
    }

    /// Menú flotante al lado del último clic. Clic afuera lo cierra.
    pub(crate) fn anchored<'a>(
        &self,
        card: impl Into<Element<'a, Message>>,
        size: Size,
        close: Message,
    ) -> Element<'a, Message> {
        let (at, window) = self.menu_at;
        let pos = crate::pointer::place(at, size, window);
        let backdrop = mouse_area(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(close.clone())
        .on_right_press(close);
        let placed = column![
            Space::new().height(pos.y),
            row![Space::new().width(pos.x), opaque(card)],
        ];
        opaque(stack![backdrop, placed])
    }

    /// Menú de la flechita de una playlist descargada.
    pub(crate) fn view_download_menu(&self, index: usize) -> Element<'_, Message> {
        let Some((p, _)) = self.playlists.get(index) else {
            return Space::new().into();
        };
        let detail = match self.downloads.get(&p.id) {
            Some((files, bytes)) => {
                let (n, size) = (
                    crate::view::songs(*files),
                    crate::view::format_bytes(*bytes),
                );
                tr!(
                    format!("{n} descargadas ({size})."),
                    format!("{n} downloaded ({size}).")
                )
            }
            None => tr!("Todavía no se bajó nada.", "Nothing downloaded yet.").to_owned(),
        };
        let card = container(
            column![
                column![
                    text(tr!("Descargada", "Downloaded")).size(theme::SIZE_M),
                    text(detail)
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                ]
                .spacing(2)
                .padding([0, 10]),
                button(
                    text(tr!("Borrar las descargas", "Delete the downloads"))
                        .size(theme::SIZE_S)
                        .style(theme::error_text)
                )
                .on_press(Message::DeleteDownloads(index))
                .width(Length::Fill)
                .padding([8, 10])
                .style(theme::list_button(false)),
            ]
            .spacing(10),
        )
        .padding(14)
        .width(PLAYLIST_MENU_WIDTH)
        .style(theme::dialog);
        self.anchored(
            card,
            Size::new(PLAYLIST_MENU_WIDTH, 120.0),
            Message::DownloadMenu(None),
        )
    }
}

const SONG_MENU_WIDTH: f32 = 340.0;
const PLAYLIST_MENU_WIDTH: f32 = 300.0;
/// Alto máximo de un menú (después, scroll).
const MENU_MAX: f32 = 460.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_remaps_indices() {
        // [a b c d], b (1) pasa a 3: [a c d b].
        let after: Vec<usize> = (0..4).map(|i| moved_index(i, 1, 3)).collect();
        assert_eq!(after, [0, 3, 1, 2]);
        // d (3) pasa a 0: [d a b c].
        let after: Vec<usize> = (0..4).map(|i| moved_index(i, 3, 0)).collect();
        assert_eq!(after, [1, 2, 3, 0]);
    }

    #[test]
    fn picks_a_free_name() {
        assert_eq!(free_name(&[]), "Mi playlist");
        assert_eq!(
            free_name(&["Mi playlist", "Mi playlist 2"]),
            "Mi playlist 3"
        );
    }
}
