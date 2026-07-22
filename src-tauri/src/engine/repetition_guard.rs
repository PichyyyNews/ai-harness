//! Streaming circuit breaker for genuine degenerate local-model output.
//!
//! llama-server is free to split one logical token into arbitrary SSE chunks.
//! Looking for repeated *chunks* therefore falsely stops normal Thai/CJK and
//! formatted answers. This guard examines the assembled Unicode text instead.

#[derive(Debug, Clone)]
pub struct RepetitionDetection {
    /// Text already sent to the frontend that must be removed from its tail.
    pub emitted_suffix: String,
    pub position: usize,
}

#[derive(Default)]
pub struct RepetitionGuard {
    recent: String,
}

impl RepetitionGuard {
    const MAX_RECENT_CHARS: usize = 2_048;
    const SINGLE_CHARACTER_REPEAT_LIMIT: usize = 9;
    const PHRASE_REPEAT_LIMIT: usize = 4;
    const MIN_PHRASE_CHARS: usize = 8;
    const MAX_PHRASE_CHARS: usize = 160;

    pub fn observe(&mut self, piece: &str) -> Option<RepetitionDetection> {
        if piece.is_empty() {
            return None;
        }
        self.recent.push_str(piece);
        self.trim_recent_window();

        let chars = self.recent.chars().collect::<Vec<_>>();
        let start = repeated_character_start(&chars)
            .or_else(|| repeated_phrase_start(&chars))?;
        let emitted_suffix = chars[start..chars.len().saturating_sub(piece.chars().count())]
            .iter()
            .collect::<String>();
        Some(RepetitionDetection {
            emitted_suffix,
            position: chars.len(),
        })
    }

    fn trim_recent_window(&mut self) {
        let char_count = self.recent.chars().count();
        if char_count <= Self::MAX_RECENT_CHARS {
            return;
        }
        self.recent = self
            .recent
            .chars()
            .skip(char_count - Self::MAX_RECENT_CHARS)
            .collect();
    }
}

fn repeated_character_start(chars: &[char]) -> Option<usize> {
    let last = *chars.last()?;
    let mut count = 0;
    for character in chars.iter().rev() {
        if *character == last {
            count += 1;
        } else {
            break;
        }
    }
    (count >= RepetitionGuard::SINGLE_CHARACTER_REPEAT_LIMIT).then_some(chars.len() - count)
}

fn repeated_phrase_start(chars: &[char]) -> Option<usize> {
    let max_width = RepetitionGuard::MAX_PHRASE_CHARS.min(chars.len() / RepetitionGuard::PHRASE_REPEAT_LIMIT);
    for width in RepetitionGuard::MIN_PHRASE_CHARS..=max_width {
        let end = chars.len();
        let pattern_start = end - width;
        let pattern = &chars[pattern_start..end];
        if pattern.iter().all(|character| character.is_whitespace()) {
            continue;
        }
        let mut repeats = 1;
        while end >= width * (repeats + 1) {
            let start = end - width * (repeats + 1);
            if chars[start..start + width] == chars[pattern_start..end] {
                repeats += 1;
            } else {
                break;
            }
        }
        if repeats >= RepetitionGuard::PHRASE_REPEAT_LIMIT {
            return Some(end - width * repeats);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::RepetitionGuard;

    #[test]
    fn catches_identical_token_loop() {
        let mut guard = RepetitionGuard::default();
        for _ in 0..8 {
            assert!(guard.observe("🙂").is_none());
        }
        let detection = guard.observe("🙂").expect("ninth repeated token should stop");
        assert_eq!(detection.emitted_suffix, "🙂".repeat(8));
    }

    #[test]
    fn catches_repeated_phrase_loop() {
        let mut guard = RepetitionGuard::default();
        assert!(guard.observe("ตอบอย่างกระชับ ").is_none());
        assert!(guard.observe("ตอบอย่างกระชับ ").is_none());
        assert!(guard.observe("ตอบอย่างกระชับ ").is_none());
        assert!(guard.observe("ตอบอย่างกระชับ ").is_some());
    }

    #[test]
    fn does_not_treat_repeated_stream_fragments_as_a_loop() {
        let mut guard = RepetitionGuard::default();
        for piece in ["การ", "ทด", "สอบ", "การ", "ทด", "สอบ", "การ", "ทด", "สอบ"] {
            assert!(guard.observe(piece).is_none());
        }
    }
}
