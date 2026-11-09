//! A run of rows of known heights inside a scroll viewport, of which Rune builds only the rows
//! near the view: `virtual_rows` with the arithmetic a caller needs to aim a scroll at a row.

use std::borrow::Cow;

use rune::{El, GrowthSpec, col};

/// A virtual list. It must be the first child of a vertical `.scroll(..)` viewport. Rows are
/// built only while they are near the view, and each row's key must be stable and unique.
pub struct Deck {
    key: String,
    rows: Vec<f32>,
    gap: f32,
}

impl Deck {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            rows: Vec::new(),
            gap: 0.,
        }
    }

    /// The rows' heights, which Rune takes as its estimates until it measures them.
    pub fn rows(mut self, rows: impl IntoIterator<Item = f32>) -> Self {
        self.rows = rows.into_iter().collect();
        self
    }

    /// The space after each row but the last, drawn as padding inside the row since virtual rows
    /// take no gaps.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// The list, keying row `i` with `key(s, i)` and building it with `draw(s, i)`.
    pub fn build<S: 'static>(
        self,
        key: impl for<'a> Fn(&'a S, usize) -> Cow<'a, str> + 'static,
        draw: impl Fn(&S, usize) -> El + 'static,
    ) -> El {
        let count = self.rows.len();
        let estimate = match count {
            0 => 1.,
            _ => self.rows.iter().sum::<f32>() / count as f32 + self.gap,
        };
        let gap = self.gap;
        col()
            .key(self.key)
            .w_full()
            .growth(GrowthSpec::default())
            .virtual_rows(count, estimate, key, move |s: &S, index| {
                let row = draw(s, index);
                match index + 1 < count {
                    true => col().w_full().pb(gap).child(row),
                    false => row,
                }
            })
    }

    /// Where each row starts, from the run's leading edge.
    pub fn tops(rows: &[f32], gap: f32) -> Vec<f32> {
        let mut top = 0.;
        rows.iter()
            .map(|height| {
                let start = top;
                top += height + gap;
                start
            })
            .collect()
    }

    /// The row under `offset` into the run, and how far into that row the offset falls.
    pub fn at(rows: &[f32], gap: f32, offset: f32) -> (usize, f32) {
        if rows.is_empty() {
            return (0, 0.);
        }
        let tops = Self::tops(rows, gap);
        let index = passed(&tops, rows, offset).min(rows.len() - 1);
        (index, (offset - tops[index]).max(0.))
    }
}

/// How many rows end at or before `top`.
fn passed(tops: &[f32], rows: &[f32], top: f32) -> usize {
    let mut low = 0;
    let mut high = tops.len();
    while low < high {
        let mid = (low + high) / 2;
        match tops[mid] + rows[mid] <= top {
            true => low = mid + 1,
            false => high = mid,
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::Deck;

    fn heights(count: usize, height: f32) -> Vec<f32> {
        (0..count).map(|_| height).collect()
    }

    #[test]
    fn tops_stack_rows_with_gaps() {
        assert_eq!(Deck::tops(&heights(3, 100.), 10.), [0., 110., 220.]);
    }

    #[test]
    fn at_finds_the_row_under_an_offset() {
        let rows = heights(4, 100.);
        assert_eq!(Deck::at(&rows, 10., 0.), (0, 0.));
        assert_eq!(Deck::at(&rows, 10., 150.), (1, 40.));
        assert_eq!(Deck::at(&rows, 10., 105.), (1, 0.));
        assert_eq!(Deck::at(&rows, 10., 9000.), (3, 8670.));
        assert_eq!(Deck::at(&[], 10., 50.), (0, 0.));
    }
}
