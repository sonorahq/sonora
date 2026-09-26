use gpui::TextAlign;

/// Returns true if the string's base direction is Right-to-Left according to
/// the Unicode Bidirectional Algorithm (UAX #9 rule P2).
///
/// UAX #9 rule P2 specifies that the paragraph base direction is determined
/// by the first character with a strong directional type (L, AL, or R),
/// skipping neutral characters (punctuation, whitespace, digits, symbols).
pub fn is_rtl(text: &str) -> bool {
    for c in text.chars() {
        if matches!(
            c,
            // Hebrew (0590-05FF), Arabic (0600-06FF), Syriac (0700-074F), Arabic Supplement (0750-077F),
            // Thaana (0780-07BF), NKo (07C0-07FF), Samaritan (0800-083F), Mandaic (0840-085F),
            // Syriac Supplement (0860-086F), Arabic Extended-B/A (0870-08FF)
            '\u{0590}'..='\u{08FF}'
            // Hebrew Presentation Forms (FB1D-FB4F), Arabic Presentation Forms-A (FB50-FDFF)
            | '\u{FB1D}'..='\u{FDFD}'
            // Arabic Presentation Forms-B (FE70-FEFC)
            | '\u{FE70}'..='\u{FEFC}'
            // RTL SMP blocks (Imperial Aramaic, Phoenician, Palmyrene, Nabataean, etc.)
            | '\u{10800}'..='\u{10FFF}'
            // Adlam, Mende Kikakui, etc.
            | '\u{1E800}'..='\u{1EFFF}'
        ) {
            return true;
        } else if c.is_alphabetic() {
            // First strong LTR character
            return false;
        }
    }
    false
}

/// Returns the natural text alignment for the text based on its paragraph base direction:
/// `TextAlign::Right` for RTL text, `TextAlign::Left` for LTR text.
pub fn natural_align(text: &str) -> TextAlign {
    if is_rtl(text) {
        TextAlign::Right
    } else {
        TextAlign::Left
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, Pixels, point, px, size};

    #[test]
    fn pure_arabic_base_direction() {
        let text = "مرحبا بالعالم";
        assert!(is_rtl(text), "Pure Arabic should have RTL base direction");
        assert_eq!(natural_align(text), TextAlign::Right);
    }

    #[test]
    fn arabic_with_english() {
        let text = "استمع إلى Music";
        assert!(
            is_rtl(text),
            "Arabic sentence with trailing English should have RTL base direction"
        );
        assert_eq!(natural_align(text), TextAlign::Right);
    }

    #[test]
    fn arabic_with_numbers() {
        let text = "أغنية رقم 25";
        assert!(
            is_rtl(text),
            "Arabic text containing numbers should have RTL base direction"
        );
        assert_eq!(natural_align(text), TextAlign::Right);
    }

    #[test]
    fn arabic_with_english_and_numbers() {
        // Starts with English 'Album', so base direction is LTR
        let text = "Album 2026 - ألبوم";
        assert!(
            !is_rtl(text),
            "Text starting with English should have LTR base direction"
        );
        assert_eq!(natural_align(text), TextAlign::Left);

        // Starts with Arabic, followed by English and numbers
        let text_arabic_first = "ألبوم 2026 - Album";
        assert!(
            is_rtl(text_arabic_first),
            "Text starting with Arabic should have RTL base direction"
        );
        assert_eq!(natural_align(text_arabic_first), TextAlign::Right);
    }

    #[test]
    fn hebrew_base_direction() {
        let text = "שלום עולם";
        assert!(is_rtl(text), "Hebrew text should have RTL base direction");
        assert_eq!(natural_align(text), TextAlign::Right);
    }

    #[test]
    fn url_direction() {
        let text = "https://example.com/أغنية";
        assert!(
            !is_rtl(text),
            "URL starting with http should have LTR base direction"
        );
        assert_eq!(natural_align(text), TextAlign::Left);
    }

    #[test]
    fn filename_path_direction() {
        let text = "/music/أغنية.mp3";
        assert!(
            !is_rtl(text),
            "File path starting with /music should have LTR base direction"
        );
        assert_eq!(natural_align(text), TextAlign::Left);
    }

    #[test]
    fn pure_ltr_regression() {
        let text = "Sonora Music Player - Volume 100%";
        assert!(
            !is_rtl(text),
            "Pure LTR string should retain LTR base direction"
        );
        assert_eq!(natural_align(text), TextAlign::Left);
    }

    #[test]
    fn logical_text_invariance() {
        // Ensure logical representation is never reversed or mutated
        let original_arabic = "مرحبا بالعالم";
        let chars: Vec<char> = original_arabic.chars().collect();
        assert_eq!(chars[0], 'م');
        assert_eq!(chars[1], 'ر');
        assert_eq!(chars[2], 'ح');
        assert_eq!(chars[3], 'ب');
        assert_eq!(chars[4], 'ا');

        // Substring and byte slicing must remain valid logical ranges
        let slice = &original_arabic[0..chars[0].len_utf8()];
        assert_eq!(slice, "م");
    }

    #[test]
    fn non_negative_selection_bounds() {
        // Simulate selection where x coordinates may be inverted in RTL runs
        fn safe_selection_bounds(bounds: Bounds<Pixels>, x1: Pixels, x2: Pixels) -> Bounds<Pixels> {
            let left_x = x1.min(x2);
            let right_x = x1.max(x2);
            Bounds::new(
                point(left_x, bounds.top()),
                size(right_x - left_x, bounds.size.height),
            )
        }

        let container = Bounds::new(point(px(10.), px(20.)), size(px(200.), px(30.)));

        // Case 1: LTR (x1 <= x2)
        let b1 = safe_selection_bounds(container, px(50.), px(120.));
        assert!(b1.size.width >= px(0.));
        assert_eq!(b1.size.width, px(70.));

        // Case 2: RTL (x1 > x2)
        let b2 = safe_selection_bounds(container, px(120.), px(50.));
        assert!(b2.size.width >= px(0.));
        assert_eq!(b2.size.width, px(70.));
        assert_eq!(b2.origin.x, px(50.));
    }
}
