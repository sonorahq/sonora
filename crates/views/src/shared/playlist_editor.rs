use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FocusHandle, Global, PathPromptOptions, Render, SharedString, Task,
    Window, div,
};
use i18n::t;
use music::Playlist;
use state::{Addition, CoverEdit, Leaving, Outcome, Shelf, Sonora, Toasts, cover_problem};
use ui::{ActiveTheme as _, Artwork, Button, Modal};
use ui::{Dismiss, FORM_CONTEXT, Input, Submit};

#[derive(Clone)]
pub(crate) enum Edit {
    Create {
        addition: Addition,
        shelf: Shelf,
    },
    /// The name and the cover of an owned playlist.
    Details(Playlist),
    Delete(Playlist),
    Again {
        playlist: Playlist,
        track: String,
        leaving: Option<Leaving>,
    },
}

pub(crate) struct PlaylistEditor {
    edit: Option<Edit>,
    name: Entity<Input>,
    /// What saving does to the cover, which the preview already shows.
    cover: CoverEdit,
    picking: Option<Task<()>>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
}

struct Installed(Entity<PlaylistEditor>);

impl Global for Installed {}

impl PlaylistEditor {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let editor = cx.new(|cx| Self {
                edit: None,
                name: cx.new(|cx| Input::new("playlist-name-placeholder", cx)),
                cover: CoverEdit::Keep,
                picking: None,
                focus: cx.focus_handle(),
                restore: None,
            });
            cx.set_global(Installed(editor));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn open(edit: Edit, window: &mut Window, cx: &mut App) {
        let editor = Self::entity(cx);
        editor.update(cx, |this, cx| this.show(edit, window, cx));
    }

    fn show(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        let name = match &edit {
            Edit::Details(playlist) => playlist.name.clone(),
            _ => String::new(),
        };
        self.restore = window.focused(cx);
        self.cover = CoverEdit::Keep;
        self.picking = None;
        self.name.update(cx, |input, cx| input.set_text(name, cx));
        match plain(&edit) {
            true => window.focus(&self.focus, cx),
            false => self.name.update(cx, |input, cx| input.focus(window, cx)),
        }
        self.edit = Some(edit);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit = None;
        self.picking = None;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Asks for a picture and shows it in the preview once it passes a quick check. A file
    /// that fails says why in a toast and leaves the cover as it was.
    fn choose_cover(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self.picking = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let checked = cx
                .background_spawn({
                    let path = path.clone();
                    async move { music::cover::check_file(&path) }
                })
                .await;
            this.update(cx, |this, cx| {
                match checked {
                    Ok(()) => this.cover = CoverEdit::Replace(path),
                    Err(unusable) => Toasts::show(Outcome::Failed, cover_problem(unusable), cx),
                }
                this.picking = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn remove_cover(&mut self, cx: &mut Context<Self>) {
        self.cover = CoverEdit::Remove;
        cx.notify();
    }

    /// The cover the preview shows: the picked file, nothing once removed, and otherwise the
    /// playlist's current cover as the library knows it.
    fn preview(&self, playlist: &Playlist, cx: &App) -> Option<SharedString> {
        match &self.cover {
            CoverEdit::Replace(path) => Some(format!("file://{}", path.display()).into()),
            CoverEdit::Remove => None,
            CoverEdit::Keep => Sonora::global(cx)
                .library
                .read(cx)
                .playlist(&playlist.id)
                .unwrap_or(playlist)
                .cover
                .clone()
                .map(Into::into),
        }
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        let name = self.name.read(cx).text().trim().to_owned();
        let library = Sonora::global(cx).library.clone();

        match edit {
            Edit::Create { addition, shelf } if !name.is_empty() => {
                library.update(cx, |library, cx| {
                    library.create_playlist(name, addition, shelf, cx);
                })
            }
            Edit::Details(playlist) => {
                let renamed = (!name.is_empty() && name != playlist.name).then_some(name);
                let cover = std::mem::replace(&mut self.cover, CoverEdit::Keep);
                library.update(cx, |library, cx| {
                    library.edit_playlist(playlist.id, renamed, cover, cx);
                })
            }
            Edit::Delete(playlist) => library.update(cx, |library, cx| {
                library.delete_playlist(playlist.id, cx);
            }),
            Edit::Again {
                playlist,
                track,
                leaving,
            } => library.update(cx, |library, cx| match leaving {
                Some(leaving) => library.move_to_playlist(playlist.id, track, leaving, cx),
                None => library.add_to_playlist(playlist.id, track, cx),
            }),
            _ => {}
        }
        cx.notify();
    }
}

fn plain(edit: &Edit) -> bool {
    matches!(edit, Edit::Delete(_) | Edit::Again { .. })
}

impl PlaylistEditor {
    /// The cover preview with its buttons beside the name field. The cover buttons show only
    /// where the provider takes a cover.
    fn details(&self, playlist: &Playlist, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let covers = Sonora::global(cx)
            .session
            .read(cx)
            .capabilities_of(Shelf::of(&playlist.id))
            .playlist_covers;
        let removable = self.cover != CoverEdit::Remove;
        div()
            .flex()
            .gap(theme.metrics.pad)
            .child(
                Artwork::new(self.preview(playlist, cx))
                    .size(theme.metrics.cover)
                    .corner_radius(theme.radius)
                    .fallback("icons/list-music.svg")
                    .flex_none(),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    // The field grows along its parent's main axis, so a row keeps it from
                    // filling the column down to the cover's height.
                    .child(div().flex().child(self.name.clone()))
                    .when(covers, |column| {
                        column.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("choose-playlist-cover")
                                        .outline()
                                        .small()
                                        .icon("icons/image.svg")
                                        .label(t!("playlist-cover-choose"))
                                        .disabled(self.picking.is_some())
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.choose_cover(cx)),
                                        ),
                                )
                                .when(removable, |row| {
                                    row.child(
                                        Button::new("remove-playlist-cover")
                                            .ghost()
                                            .small()
                                            .icon("icons/image-off.svg")
                                            .label(t!("playlist-cover-remove"))
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.remove_cover(cx)),
                                            ),
                                    )
                                }),
                        )
                    }),
            )
    }
}

impl Render for PlaylistEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(edit) = self.edit.clone() else {
            return div().into_any_element();
        };
        let theme = *cx.theme();
        let deleting = matches!(edit, Edit::Delete(_));
        let asking = plain(&edit);
        let title = match &edit {
            Edit::Create { .. } => t!("playlist-create-title"),
            Edit::Details(_) => t!("playlist-edit-title"),
            Edit::Delete(_) => t!("playlist-delete-title"),
            Edit::Again { .. } => t!("playlist-again-title"),
        };
        let detail = match &edit {
            Edit::Delete(playlist) => Some(t!("playlist-delete-confirm", name = &playlist.name)),
            Edit::Again { playlist, .. } => {
                Some(t!("playlist-again-confirm", name = &playlist.name))
            }
            _ => None,
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.apply(window, cx);
            }))
            .child(
                Modal::new("playlist-editor", title)
                    .w(theme.metrics.cover * 2.8)
                    .when_some(detail, Modal::detail)
                    .map(|modal| match &edit {
                        Edit::Details(playlist) => modal.child(self.details(playlist, cx)),
                        _ if asking => modal,
                        _ => modal.child(self.name.clone()),
                    })
                    .action(
                        Button::new("cancel-playlist-edit")
                            .ghost()
                            .label(t!("common-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .action(
                        Button::new("apply-playlist-edit")
                            .when_else(
                                deleting,
                                |button| button.destructive(),
                                |button| button.primary(),
                            )
                            .label(match &edit {
                                Edit::Delete(_) => t!("common-delete"),
                                Edit::Again { .. } => t!("playlist-again-add"),
                                _ => t!("common-save"),
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}
