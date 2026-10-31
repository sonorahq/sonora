use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, Context, DragMoveEvent, Pixels, Point, Render, SharedString, Window, div};
use i18n::t;
use music::WritingSystem;
use router::{Destination, SettingsPage, navigate};
use state::{AppSettings, LYRICS_SOURCES};
use ui::{
    ActiveTheme as _, Button, Ghost, MAX_LYRICS_SCALE, MIN_LYRICS_SCALE, MenuItem, Picker, Switch,
    drop_gap, drop_marker, grip, tabular,
};

use super::rows::drill;
use super::{Group, SettingsView, Slot};

/// A lyrics source being dragged within the priority list. It draws as a chip under the cursor
/// and changes the saved order only when dropped.
#[derive(Clone)]
pub(super) struct DraggedSource {
    provider: String,
    label: SharedString,
    /// Where the cursor grabbed the entry, so the chip starts under it.
    position: Point<Pixels>,
}

impl SettingsView {
    pub(super) fn lyrics_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-group-display", "icons/mic-vocal.svg").rows([
                Slot::Karaoke,
                Slot::BlurLyrics,
                Slot::Romanized,
                Slot::PanelLyricsSize,
                Slot::FullscreenLyricsSize,
            ]),
            Group::new("settings-group-sources", "icons/list-music.svg")
                .rows([Slot::LyricsSources, Slot::LyricsForLocal]),
        ]
    }

    pub(super) fn lyrics_sources_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-group-lyrics-order", "icons/arrow-up-down.svg")
                .rows([Slot::LyricsPriority]),
        ]
    }

    pub(super) fn panel_lyrics_size_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.settings.read(cx).panel_lyrics_scale();

        self.lyrics_size_row(
            "panel-lyrics-size",
            "settings-panel-lyrics-size",
            scale,
            |settings, scale, cx| settings.set_panel_lyrics_scale(scale, cx),
            cx,
        )
    }

    pub(super) fn fullscreen_lyrics_size_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.settings.read(cx).fullscreen_lyrics_scale();

        self.lyrics_size_row(
            "fullscreen-lyrics-size",
            "settings-fullscreen-lyrics-size",
            scale,
            |settings, scale, cx| settings.set_fullscreen_lyrics_scale(scale, cx),
            cx,
        )
    }

    /// A lyrics text size, stepped a tenth at a time between its limits.
    pub(super) fn lyrics_size_row(
        &self,
        id: &'static str,
        title: &'static str,
        scale: f32,
        apply: fn(&mut AppSettings, f32, &mut Context<AppSettings>),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step =
            move |suffix: &'static str, label: &'static str, name: &'static str, delta: f32| {
                let wanted = (scale + delta).clamp(MIN_LYRICS_SCALE, MAX_LYRICS_SCALE);

                Button::new(SharedString::from(format!("{id}-{suffix}")))
                    .label(label)
                    .name(i18n::lookup(name, None))
                    .small()
                    .outline()
                    .disabled(wanted == scale)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| apply(settings, wanted, cx));
                        cx.notify();
                    }))
            };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("smaller", "\u{2212}", "settings-font-smaller", -0.1))
            .child(ui::spoken(
                SharedString::from(format!("{id}-value")),
                t!(
                    "settings-lyrics-size-value",
                    size = (scale * 100.).round() as i64
                ),
            ))
            .child(step("larger", "+", "settings-font-larger", 0.1));

        self.row(i18n::lookup(title, None), actions.into_any_element())
    }

    pub(super) fn lyrics_for_local_files_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).lyrics_for_local_files();

        self.row(
            t!("settings-lyrics-for-local-files"),
            Switch::new("lyrics-for-local-files", on)
                .name(t!("settings-lyrics-for-local-files"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_lyrics_for_local_files(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    /// The row that opens the sources page, saying how many sources are on.
    pub(super) fn lyrics_sources_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let count = LYRICS_SOURCES
            .iter()
            .filter(|provider| settings.lyrics_provider_enabled(provider))
            .count();

        drill(
            "lyrics-sources",
            t!("settings-lyrics-sources"),
            Some(t!("settings-lyrics-providers-selected", count = count)),
            cx,
        )
        .on_click(|_, _, cx| navigate(Destination::Settings(SettingsPage::LyricsSources), cx))
        .into_any_element()
    }

    /// Every lyrics source in one list. The ones that are on come first, in the order ties
    /// between them are settled, and each can be dragged anywhere among them. The ones that are
    /// off follow. Every source carries its own switch.
    pub(super) fn lyrics_priority(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let order = settings.enabled_lyrics_provider_priority();
        let off = LYRICS_SOURCES
            .into_iter()
            .filter(|provider| !settings.lyrics_provider_enabled(provider))
            .collect::<Vec<_>>();
        let count = order.len();
        let total = count + off.len();
        let mut entries: Vec<AnyElement> = order
            .into_iter()
            .enumerate()
            .map(|(index, provider)| self.lyrics_source_entry(index, provider, count, total, cx))
            .collect();
        entries.extend(
            off.into_iter().enumerate().map(|(index, provider)| {
                self.lyrics_source_off((count + index, total), provider, cx)
            }),
        );

        div()
            .id("lyrics-priority")
            .role(Role::List)
            .aria_label(t!("settings-group-lyrics-order"))
            .flex()
            .flex_col()
            .children(entries)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<DraggedSource>, _, cx| {
                    if !event.bounds.contains(&event.event.position)
                        && this.lyrics_drop_gap.take().is_some()
                    {
                        cx.notify();
                    }
                }),
            )
            .on_drop(cx.listener(|this, dragged: &DraggedSource, _, cx| {
                let Some(gap) = this.lyrics_drop_gap.take() else {
                    return;
                };
                this.settings.update(cx, |settings, cx| {
                    settings.place_lyrics_provider(&dragged.provider, gap, cx)
                });
                cx.notify();
            }))
            .into_any_element()
    }

    /// The switch that turns one source on or off.
    fn lyrics_source_switch(
        &self,
        provider: &'static str,
        on: bool,
        cx: &mut Context<Self>,
    ) -> Switch {
        Switch::new(
            SharedString::from(format!("lyrics-source-on-{provider}")),
            on,
        )
        .name(source_name(provider))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.settings.update(cx, |settings, cx| {
                settings.set_lyrics_provider(provider, !on, cx)
            });
        }))
    }

    /// A source that is off, placed by its index and the list's length: no place in the order,
    /// its name muted, and its switch.
    fn lyrics_source_off(
        &self,
        (index, total): (usize, usize),
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();

        div()
            .id(SharedString::from(format!("lyrics-source-{provider}")))
            .role(Role::ListItem)
            .aria_label(source_name(provider))
            .aria_position_in_set(index + 1)
            .aria_size_of_set(total)
            .flex()
            .items_center()
            .gap_3()
            .h(theme.metrics.list_row)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(source_name(provider)),
            )
            .child(self.lyrics_source_switch(provider, false, cx))
            .into_any_element()
    }

    /// One source that is on: the grip it is dragged by, its muted place in the order, its name
    /// and its switch. The whole row drags, the grip only shows that it does. `count` sources are
    /// on out of the `total` the list shows.
    pub(super) fn lyrics_source_entry(
        &self,
        index: usize,
        provider: String,
        count: usize,
        total: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let name = source_name(&provider);
        let id = SharedString::from(format!("lyrics-source-{provider}"));
        let edge = match self.lyrics_drop_gap {
            Some(gap) if gap == index => Some(ui::Edge::Above),
            Some(gap) if gap == count && index + 1 == count => Some(ui::Edge::Below),
            _ => None,
        };
        let switch = LYRICS_SOURCES
            .into_iter()
            .find(|source| *source == provider)
            .map(|source| self.lyrics_source_switch(source, true, cx));
        let dragged = DraggedSource {
            provider,
            label: name.clone(),
            position: Point::default(),
        };
        div()
            .id(id)
            .role(Role::ListItem)
            .aria_label(name.clone())
            .aria_position_in_set(index + 1)
            .aria_size_of_set(total)
            .relative()
            .h(theme.metrics.list_row)
            .flex()
            .items_center()
            .gap_3()
            .cursor_grab()
            .child(grip(cx))
            .child(
                div()
                    .flex_none()
                    .font_features(tabular())
                    .text_color(theme.muted_foreground)
                    .child(SharedString::from((index + 1).to_string())),
            )
            .child(div().flex_1().min_w_0().truncate().child(name))
            .children(switch)
            .on_drag(dragged, |dragged, position, _, cx| {
                cx.new(|_| DraggedSource {
                    position,
                    ..dragged.clone()
                })
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DraggedSource>, _, cx| {
                    // The entries touch, so only the one under the cursor answers, and the list
                    // clears the gap once the cursor leaves it.
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    let Some(gap) = drop_gap(event.bounds, event.event.position, index) else {
                        return;
                    };
                    let order = this.settings.read(cx).enabled_lyrics_provider_priority();
                    let from = order
                        .iter()
                        .position(|name| *name == event.drag(cx).provider);
                    let gap = Some(gap)
                        .filter(|gap| from.is_some_and(|from| *gap != from && *gap != from + 1));
                    if this.lyrics_drop_gap != gap {
                        this.lyrics_drop_gap = gap;
                        cx.notify();
                    }
                }),
            )
            .when_some(edge, |entry, edge| entry.child(drop_marker(edge, cx)))
            .into_any_element()
    }

    pub(super) fn karaoke_lyrics_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).karaoke_lyrics();

        self.row(
            t!("settings-karaoke-lyrics"),
            Switch::new("karaoke-lyrics", on)
                .name(t!("settings-karaoke-lyrics"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_karaoke_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn blur_lyrics_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).blur_lyrics();

        self.row(
            t!("settings-blur-lyrics"),
            Switch::new("blur-lyrics", on)
                .name(t!("settings-blur-lyrics"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn romanized_lyrics_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let on = settings.romanized_lyrics();
        let scripts = settings.romanization_scripts();
        let picker = Picker::new(
            "romanization-scripts",
            &self.popovers,
            t!("settings-romanization-writing-systems"),
        )
        .width(Picker::REGULAR)
        .sticky()
        .items(WritingSystem::ALL.map(|writing_system| {
            let (id, label) = romanization_script_copy(writing_system);
            let selected = scripts.contains(writing_system);
            MenuItem::new(id, i18n::lookup(label, None))
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_romanization_script(writing_system, !selected, cx);
                    });
                }))
        }));
        let action = div()
            .flex()
            .items_center()
            .gap_2()
            .when(on, |this| this.child(picker))
            .child(
                Switch::new("romanized-lyrics", on)
                    .name(t!("settings-romanized-lyrics"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_romanized_lyrics(!on, cx);
                        });
                    })),
            );

        self.row(t!("settings-romanized-lyrics"), action.into_any_element())
    }
}

pub(super) fn romanization_script_copy(
    writing_system: WritingSystem,
) -> (&'static str, &'static str) {
    match writing_system {
        WritingSystem::Japanese => ("romanization-japanese", "settings-romanization-japanese"),
        WritingSystem::Chinese => ("romanization-chinese", "settings-romanization-chinese"),
        WritingSystem::Korean => ("romanization-korean", "settings-romanization-korean"),
        WritingSystem::Cyrillic => ("romanization-cyrillic", "settings-romanization-cyrillic"),
        WritingSystem::Greek => ("romanization-greek", "settings-romanization-greek"),
        WritingSystem::Arabic => ("romanization-arabic", "settings-romanization-arabic"),
        WritingSystem::Other => ("romanization-other", "settings-romanization-other"),
    }
}

impl Render for DraggedSource {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Ghost::new(self.position, self.label.clone())
    }
}

/// The name a lyrics source goes by in the settings, falling back to the provider's own name.
pub(super) fn source_name(provider: &str) -> SharedString {
    let key = match provider {
        music::lyrics::LOCAL => "settings-lyrics-provider-local",
        "Subsonic" => "settings-lyrics-provider-subsonic",
        "Spotify" => "settings-lyrics-provider-spotify",
        "YouTube Music" => "settings-lyrics-provider-youtube",
        "Apple Music" => "settings-lyrics-provider-apple-music",
        "Musixmatch" => "settings-lyrics-provider-musixmatch",
        "LrcLib" => "settings-lyrics-provider-lrclib",
        "Kugou" => "settings-lyrics-provider-kugou",
        "NetEase" => "settings-lyrics-provider-netease",
        _ => return SharedString::from(provider.to_owned()),
    };
    i18n::lookup(key, None)
}
