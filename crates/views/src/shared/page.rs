//! What the list pages share around their lists: the width they lay out in, and keeping a
//! list's filters in settings.

use std::collections::HashMap;

use rune::Cx;
use state::FilterValue;
use ui::{Filter, FilterChange, Theme};

use crate::Ui;

/// The width of the content area between the sidebars less `inset`, never under the narrowest
/// a page lays out at. The view runs again as it changes.
pub(crate) fn content_width(cx: &Cx<Ui>, inset: f32) -> f32 {
    let width = cx
        .layout_with("content", |rect| rect[2].round())
        .unwrap_or(0.);
    (width - inset).max(ui::MIN_CONTENT)
}

/// `n` rems in points: GPUI measured its spacing in rems, one rem being the theme's font size.
pub(crate) fn rems(theme: &Theme, n: f32) -> f32 {
    theme.font_size * n
}

/// The room a page with `inset` on both sides leaves its content, less a hairline either side.
pub(crate) fn reserved(inset: f32) -> f32 {
    inset * 2. + 2.
}

/// Merges the current axes into the stored ones: a narrowed axis upserts its key, a whole one
/// drops it, and an axis the list does not name right now leaves storage alone. That last rule
/// keeps a saved year span alive while its rows are still loading, and resetting a list clears
/// its entry on the next store.
pub(crate) fn snapshot(
    axes: Vec<Filter>,
    into: HashMap<String, FilterValue>,
) -> HashMap<String, FilterValue> {
    let mut stored = into;
    for axis in axes {
        match axis {
            Filter::Range(axis) if axis.whole() => {
                stored.remove(axis.key);
            }
            Filter::Range(axis) => {
                stored.insert(
                    axis.key.to_owned(),
                    FilterValue::Range(axis.value.0, axis.value.1),
                );
            }
            Filter::Flag(axis) if axis.on => {
                stored.insert(axis.key.to_owned(), FilterValue::Flag(true));
            }
            Filter::Flag(axis) => {
                stored.remove(axis.key);
            }
        }
    }
    stored
}

/// The changes that put back the stored axes a list has not narrowed itself yet. Ranges clamp
/// to the current bounds, so a span saved before its rows arrived still lands, while one the
/// library outgrew stays whole instead of filtering everything out. Stored keys the list no
/// longer names are ignored.
pub(crate) fn restore(
    stored: &HashMap<String, FilterValue>,
    axes: Vec<Filter>,
) -> Vec<FilterChange> {
    let mut changes = Vec::new();
    for axis in axes {
        match axis {
            Filter::Range(axis) => {
                let Some(FilterValue::Range(low, high)) = stored.get(axis.key) else {
                    continue;
                };
                let value = (
                    low.clamp(axis.bounds.0, axis.bounds.1),
                    high.clamp(axis.bounds.0, axis.bounds.1),
                );
                if value != axis.bounds {
                    changes.push(FilterChange::Range(axis.key, value));
                }
            }
            Filter::Flag(axis) => {
                let Some(FilterValue::Flag(on)) = stored.get(axis.key) else {
                    continue;
                };
                if *on != axis.on {
                    changes.push(FilterChange::Flag(axis.key, *on));
                }
            }
        }
    }
    changes
}
