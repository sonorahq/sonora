//! Moving one track within a playlist's order, for the providers that let the listener reorder.
//!
//! The listener moves a row of the listing a provider answered, which can leave out entries
//! the provider still keeps, such as a missing file or an episode. A provider maps the move
//! onto its own full order with [`placement`] before it writes it.

use anyhow::{Result, bail};

/// Where a move puts an entry in a provider's full order, counted before the move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The entry being moved.
    pub from: usize,
    /// The entry it goes in front of. The length of the order puts it at the end.
    pub before: usize,
}

impl Placement {
    /// The entry's index once it has moved, counted in the order after the move.
    pub fn to(self) -> usize {
        match self.before > self.from {
            true => self.before - 1,
            false => self.before,
        }
    }
}

/// Moves the item at `from` so it ends up at `to`, counted in the list after the move.
/// Answers `false`, leaving the list alone, when either is out of range or the two are equal.
pub fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= items.len() || to >= items.len() || from == to {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

/// Maps a move of the listed row `from` to `to` onto the full order. `listed` holds, for each
/// listed row in order, its index in the full order. The moved entry goes in front of the
/// listed row that follows it after the move, or straight after the last listed row when
/// nothing does, so an entry the listing leaves out keeps its neighbours.
pub fn placement(listed: &[usize], from: usize, to: usize) -> Result<Placement> {
    if from >= listed.len() || to >= listed.len() {
        bail!("the move is outside the playlist");
    }
    if from == to {
        bail!("the track is already there");
    }
    let mut after: Vec<usize> = listed.to_vec();
    let moved = after.remove(from);
    let before = match after.get(to) {
        Some(&next) => next,
        None => after.last().map_or(moved, |&last| last + 1),
    };
    Ok(Placement {
        from: moved,
        before,
    })
}

/// Checks that the provider still lists exactly the tracks the listener saw, in the same
/// order. A move counts places in what the listener saw, so a playlist that gained, lost or
/// reordered a track elsewhere moves nothing rather than landing the track somewhere else.
pub fn confirm(listed: &[Option<&str>], seen: &[String]) -> Result<()> {
    let same = listed.len() == seen.len()
        && listed
            .iter()
            .zip(seen)
            .all(|(listed, seen)| *listed == Some(seen.as_str()));
    match same {
        true => Ok(()),
        false => bail!("the playlist changed before the move"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_an_item_down_and_up() {
        let mut items = vec!["a", "b", "c", "d"];
        assert!(move_item(&mut items, 0, 2));
        assert_eq!(items, ["b", "c", "a", "d"]);
        assert!(move_item(&mut items, 3, 0));
        assert_eq!(items, ["d", "b", "c", "a"]);
    }

    #[test]
    fn leaves_the_list_alone_for_an_empty_or_outside_move() {
        let mut items = vec!["a", "b"];
        assert!(!move_item(&mut items, 1, 1));
        assert!(!move_item(&mut items, 2, 0));
        assert!(!move_item(&mut items, 0, 2));
        assert_eq!(items, ["a", "b"]);
    }

    #[test]
    fn places_a_move_in_front_of_the_next_listed_row() {
        // a full order of five where 1 and 3 are not listed
        let listed = [0, 2, 4];
        let down = placement(&listed, 0, 1).unwrap();
        assert_eq!(down, Placement { from: 0, before: 4 });
        assert_eq!(down.to(), 3);

        let up = placement(&listed, 2, 0).unwrap();
        assert_eq!(up, Placement { from: 4, before: 0 });
        assert_eq!(up.to(), 0);
    }

    #[test]
    fn places_a_move_to_the_end_after_the_last_listed_row() {
        let listed = [0, 1, 2];
        let end = placement(&listed, 0, 2).unwrap();
        assert_eq!(end, Placement { from: 0, before: 3 });
        assert_eq!(end.to(), 2);

        // it lands straight after the last listed row, wherever that sits in the full order
        let gapped = [0, 1, 3];
        assert_eq!(placement(&gapped, 0, 2).unwrap().before, 4);
        assert_eq!(placement(&[0, 1, 2, 5][..], 0, 3).unwrap().before, 6);
    }

    #[test]
    fn refuses_an_empty_or_outside_move() {
        assert!(placement(&[0, 1], 1, 1).is_err());
        assert!(placement(&[0, 1], 2, 0).is_err());
    }

    fn seen(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn confirms_the_listing_the_listener_saw() {
        let listed = [Some("a"), Some("b"), Some("c")];
        assert!(confirm(&listed, &seen(&["a", "b", "c"])).is_ok());
    }

    #[test]
    fn refuses_a_listing_that_changed_elsewhere() {
        // a track added after the moved one would shift where the move lands
        let grown = [Some("a"), Some("x"), Some("b"), Some("c")];
        assert!(confirm(&grown, &seen(&["a", "b", "c"])).is_err());
        let reordered = [Some("b"), Some("a"), Some("c")];
        assert!(confirm(&reordered, &seen(&["a", "b", "c"])).is_err());
        let unnamed = [Some("a"), None];
        assert!(confirm(&unnamed, &seen(&["a", ""])).is_err());
    }
}
