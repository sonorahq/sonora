use std::rc::Rc;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Context, Pixels, SharedString, Window, div};
use i18n::t;
use music::{Ranked, Track};
use router::Destination;
use state::{Entry, Found, Origin, StatsSource};
use ui::{Card, Corner};

use super::{Layout, StatsView};
use crate::shared::album_grid::CardGrid;
use crate::shared::shelves::RailSpec;
use crate::shared::track_card::{StartHandler, TrackCard};
use crate::shared::{cards, cells};

/// A ranking the page lists as a rail of cards.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Artists,
    Songs,
    Albums,
    Songwriters,
    Producers,
}

impl Kind {
    pub(super) const ALL: [Self; 5] = [
        Self::Artists,
        Self::Songs,
        Self::Albums,
        Self::Songwriters,
        Self::Producers,
    ];
    /// The rails every source has, which stand in for the page while it loads.
    pub(super) const MAIN: [Self; 3] = [Self::Artists, Self::Songs, Self::Albums];

    fn tag(self) -> &'static str {
        match self {
            Self::Artists => "stats-top-artists",
            Self::Songs => "stats-top-songs",
            Self::Albums => "stats-top-albums",
            Self::Songwriters => "stats-top-songwriters",
            Self::Producers => "stats-top-producers",
        }
    }

    /// The heading, which is the message the tag names.
    pub(super) fn title(self) -> SharedString {
        i18n::lookup(self.tag(), None)
    }
}

/// What the cards of a songs rail play from.
struct Queue {
    /// Only the songs that can play. A chart entry the catalogue has not matched has no id.
    tracks: Rc<Vec<Track>>,
    /// Where each ranked song sits in `tracks`, or nothing for one that is left out.
    places: Vec<Option<usize>>,
    start: StartHandler,
}

/// A card for a ranked item that has no catalogue page of its own.
struct Plain {
    title: String,
    subtitle: SharedString,
    cover: Option<String>,
    icon: &'static str,
    circle: bool,
    entry: Option<Entry>,
}

impl StatsView {
    /// A ranking in a sideways rail with paging arrows, or nothing when it is empty.
    pub(super) fn rail(
        &self,
        kind: Kind,
        layout: &Layout<'_>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let data = self.stats.read(cx).data();
        let count = match kind {
            Kind::Artists => data.artists.len(),
            Kind::Songs => data.tracks.len(),
            Kind::Albums => data.albums.len(),
            Kind::Songwriters => data.songwriters.len(),
            Kind::Producers => data.producers.len(),
        };
        if count == 0 {
            return None;
        }

        let tile = CardGrid::layout(layout.width).card;
        let queue = (kind == Kind::Songs).then(|| {
            let mut playable = Vec::new();
            let places = data
                .tracks
                .iter()
                .map(|ranked| {
                    ranked.item.id.is_some().then(|| {
                        playable.push(ranked.item.clone());
                        playable.len() - 1
                    })
                })
                .collect();
            let tracks = Rc::new(playable);
            let (playback, queued) = (self.playback.clone(), tracks.clone());
            let start: StartHandler = Rc::new(move |place, cx| {
                playback.update(cx, |playback, cx| {
                    playback.start(queued.as_ref().clone(), place, None, cx)
                });
            });
            Queue {
                tracks,
                places,
                start,
            }
        });
        let weak = cx.entity().downgrade();
        Some(self.rails[kind as usize].render(
            RailSpec {
                tag: kind.tag(),
                place: 0,
                title: kind.title(),
                count,
                tile,
                columns: layout.columns.max(1),
                tabs: None,
                note: self.notes(kind, cx),
            },
            layout.window,
            cx,
            layout.notify,
            move |at, _, cx| {
                let Some(view) = weak.upgrade() else {
                    return div().into_any_element();
                };
                let Some((id, card)) = view.read(cx).card(kind, at, tile, queue.as_ref(), cx)
                else {
                    return div().into_any_element();
                };
                div()
                    .id(id)
                    .role(Role::ListItem)
                    .aria_label(t!("stats-rank", rank = at + 1))
                    .aria_position_in_set(at + 1)
                    .aria_size_of_set(count)
                    .child(card)
                    .into_any_element()
            },
        ))
    }

    /// What the rail counts, as badges beside its heading: how many, and how many are new.
    fn notes(&self, kind: Kind, cx: &App) -> Option<AnyElement> {
        let data = self.stats.read(cx).data();
        let o = &data.overview;
        // A source with no totals still says how many its rail lists.
        let listed = |count: usize| Some(count).filter(|count| *count > 0);
        let (total, new, extras) = match kind {
            Kind::Artists => (
                o.artists
                    .or_else(|| listed(data.artists.len()))
                    .map(|count| t!("count-artists", count = count)),
                o.new_artists,
                vec![
                    o.collab_share
                        .map(|share| t!("stats-badge-collabs", share = share)),
                ],
            ),
            Kind::Songs => (
                o.tracks
                    .or_else(|| listed(data.tracks.len()))
                    .map(|count| t!("count-songs", count = count)),
                o.new_tracks,
                vec![
                    o.plays.map(|plays| {
                        let value = cells::compact(plays as u64);
                        t!("stats-badge-plays", count = plays, value = value.as_str())
                    }),
                    o.avg_length.map(|secs| {
                        let length = ui::clock(std::time::Duration::from_secs(secs)).to_string();
                        t!("stats-badge-length", length = length)
                    }),
                    o.mainstream
                        .map(|score| t!("stats-badge-mainstream", score = score)),
                ],
            ),
            Kind::Songwriters | Kind::Producers => (None, None, Vec::new()),
            Kind::Albums => (
                o.albums
                    .or_else(|| listed(data.albums.len()))
                    .map(|count| t!("count-albums", count = count)),
                None,
                Vec::new(),
            ),
        };
        self.badges(
            [
                total.map(|text| (text, false)),
                new.map(|count| (t!("stats-badge-new", count = count), true)),
            ]
            .into_iter()
            .chain(
                extras
                    .into_iter()
                    .map(|text| text.map(|text| (text, false))),
            ),
            cx,
        )
    }

    /// The card at rank `at + 1`, under an id that stays with its item as the ranking changes.
    fn card(
        &self,
        kind: Kind,
        at: usize,
        tile: Pixels,
        queue: Option<&Queue>,
        cx: &App,
    ) -> Option<(SharedString, AnyElement)> {
        let stats = self.stats.read(cx);
        let data = stats.data();
        let charted = matches!(stats.source(), StatsSource::Scrobbler(_));
        let (id, standing, card) = match kind {
            Kind::Artists | Kind::Songwriters | Kind::Producers => {
                let list = match kind {
                    Kind::Songwriters => &data.songwriters,
                    Kind::Producers => &data.producers,
                    _ => &data.artists,
                };
                let ranked = list.get(at)?;
                let artist = &ranked.item;
                let id = format!(
                    "{}-{}",
                    kind.tag(),
                    identity(&artist.id, &ranked.external_url, &artist.name)
                );
                let card = match artist.id.is_empty() {
                    false => cards::artist_card(card_id(&id), artist, &self.playback, cx),
                    true => self.plain(
                        &id,
                        Plain {
                            title: artist.name.clone(),
                            subtitle: i18n::lookup("artist-eyebrow", None),
                            cover: artist.cover.clone(),
                            icon: "icons/user-round.svg",
                            circle: true,
                            entry: charted.then(|| Entry::Artist {
                                name: artist.name.clone(),
                            }),
                        },
                    ),
                };
                (id, standing(at + 1, ranked, 0), card)
            }
            Kind::Songs => {
                let ranked = data.tracks.get(at)?;
                let track = &ranked.item;
                let id = format!(
                    "stats-track-{}-{}",
                    identity(
                        track.id.as_deref().unwrap_or_default(),
                        &ranked.external_url,
                        &track.name
                    ),
                    track.artists
                );
                let seat = queue.and_then(|queue| Some((queue, (*queue.places.get(at)?)?)));
                let card = match seat {
                    Some((queue, place)) => TrackCard::new(
                        "stats-track-card",
                        place,
                        queue.tracks.clone(),
                        self.playback.clone(),
                    )
                    .start(Some(queue.start.clone()))
                    .render(cx)
                    .underline()
                    .press({
                        let id = track.id.clone().unwrap_or_default();
                        move |_, _, cx| router::navigate(Destination::Song(id.clone().into()), cx)
                    }),
                    _ => self.plain(
                        &id,
                        Plain {
                            title: track.name.clone(),
                            subtitle: track.artists.clone().into(),
                            cover: track.cover.clone(),
                            icon: "icons/music.svg",
                            circle: false,
                            entry: charted.then(|| Entry::Track {
                                title: track.name.clone(),
                                artist: track.artists.clone(),
                            }),
                        },
                    ),
                };
                (id, standing(at + 1, ranked, track.popularity), card)
            }
            Kind::Albums => {
                let ranked = data.albums.get(at)?;
                let album = &ranked.item;
                let id = format!(
                    "stats-album-{}-{}",
                    identity(&album.id, &ranked.external_url, &album.name),
                    album.artists
                );
                let card = match album.id.is_empty() {
                    false => cards::album_card(card_id(&id), album, &self.playback, cx),
                    true => self.plain(
                        &id,
                        Plain {
                            title: album.name.clone(),
                            subtitle: album.artists.clone().into(),
                            cover: album.cover.clone(),
                            icon: "icons/music.svg",
                            circle: false,
                            entry: charted.then(|| Entry::Album {
                                title: album.name.clone(),
                                artist: album.artists.clone(),
                            }),
                        },
                    ),
                };
                (id, standing(at + 1, ranked, 0), card)
            }
        };
        let card = standing.dress(card.tile(tile).flat());
        Some((SharedString::from(id), card.into_any_element()))
    }

    /// A card for an item the active catalogue has not matched. A chart entry opens and plays
    /// by being matched on demand.
    fn plain(&self, id: &str, plain: Plain) -> Card {
        let Plain {
            title,
            subtitle,
            cover,
            icon,
            circle,
            entry,
        } = plain;
        Card::new(card_id(id), title)
            .cover(cover)
            .fallback(icon)
            .description(subtitle.clone())
            .meta(subtitle)
            .when(circle, Card::circle)
            .when_some(entry, |card, entry| {
                card.press(self.open(entry.clone(), false))
                    .play(false, self.open(entry, true))
            })
    }

    /// Matches a chart entry to the catalogue, then opens its page, or starts it playing from the
    /// card's play button. A song plays as a radio.
    fn open(
        &self,
        entry: Entry,
        play: bool,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let (stats, playback) = (self.stats.clone(), self.playback.clone());
        move |_, _, cx| {
            let playback = playback.clone();
            stats.update(cx, |stats, cx| {
                stats.resolve(
                    entry.clone(),
                    move |found, cx| {
                        let Some(found) = found else {
                            return;
                        };
                        let origin = match (&found, play) {
                            (Found::Track(track), true) => {
                                return playback
                                    .update(cx, |playback, cx| playback.play_radio(track, cx));
                            }
                            (Found::Artist(artist), true) => {
                                Origin::artist(artist.id.clone()).named(artist.name.clone())
                            }
                            (Found::Album(album), true) => {
                                Origin::album(album.id.clone()).named(album.name.clone())
                            }
                            (_, false) => {
                                return router::navigate(
                                    match found {
                                        Found::Track(track) => {
                                            Destination::Song(track.id.unwrap_or_default().into())
                                        }
                                        Found::Artist(artist) => {
                                            Destination::Artist(artist.id.into())
                                        }
                                        Found::Album(album) => Destination::Album(album.id.into()),
                                    },
                                    cx,
                                );
                            }
                        };
                        playback.update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
                    },
                    cx,
                );
            });
        }
    }
}

/// What the source knows of a ranked item: its place, plays, whether it is new and how popular
/// the song is. A source that ranks by affinity alone has no plays, and none are invented.
struct Standing {
    rank: usize,
    count: Option<usize>,
    fresh: bool,
    popularity: u32,
}

fn standing<T>(rank: usize, ranked: &Ranked<T>, popularity: u32) -> Standing {
    Standing {
        rank,
        count: ranked.count,
        fresh: ranked.fresh,
        popularity,
    }
}

impl Standing {
    /// The word that marks an item as new to the ranking. A rank that only moved has none.
    fn movement(&self) -> Option<SharedString> {
        self.fresh.then(|| t!("stats-rank-new"))
    }

    /// Lays the standing over the card's artwork: the place, with how it moved, in the top left,
    /// the song's popularity in the bottom left and the plays in the bottom right, where the play
    /// button takes over on hover. The card's spoken description carries all of it.
    fn dress(self, card: Card) -> Card {
        let place = t!("stats-rank", rank = self.rank);
        let movement = self.movement();
        let plays = self.count.map(|count| {
            let value = cells::compact(count as u64);
            t!("stats-badge-plays", count = count, value = value.as_str())
        });
        let spoken = [
            Some(place.clone()),
            movement.clone(),
            plays.clone(),
            (self.popularity > 0).then(|| t!("stats-rank-popularity", score = self.popularity)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let place = match movement {
            Some(movement) => SharedString::from(format!("{place} {movement}")),
            None => place,
        };
        card.description(spoken)
            .badge(Corner::TopLeft, place)
            .when(self.popularity > 0, |card| {
                card.icon_badge(
                    Corner::BottomLeft,
                    "icons/flame.svg",
                    self.popularity.to_string(),
                )
            })
            .when_some(plays, |card, plays| card.badge(Corner::BottomRight, plays))
    }
}

/// What keeps a row's id steady as the ranking changes: the catalogue id when it has one, else
/// the chart's own page, else the name.
fn identity<'a>(id: &'a str, url: &'a Option<String>, name: &'a str) -> &'a str {
    match id.is_empty() {
        true => url.as_deref().unwrap_or(name),
        false => id,
    }
}

/// The id of the card in the row that has `row` for its id.
fn card_id(row: &str) -> SharedString {
    SharedString::from(format!("{row}-card"))
}
