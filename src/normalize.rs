//! Unicode NFC normalization utilities.
//!
//! The macOS filesystem/input method often stores Korean characters in NFD (decomposed) form.
//! To prevent search misses, we force both collected text and user inputs into NFC (composed) form.

use unicode_normalization::UnicodeNormalization;

/// Normalizes a string to Unicode NFC form.
pub fn nfc(input: &str) -> String {
    input.nfc().collect()
}

/// Performs NFC normalization followed by lowercasing for search comparison (case-insensitive matching).
pub fn nfc_lower(input: &str) -> String {
    input.nfc().collect::<String>().to_lowercase()
}
