//! Low-cost streaming circuit breaker for degenerate local-model output.
//!
//! llama-server streams text fragments rather than tokenizer IDs.  The guard
//! deliberately works on those fragments: it catches the common failure modes
//! (a repeated emoji/token or a short repeated sequence) without introducing a
//! second tokenizer or delaying the stream.

#[derive(Debug, Clone)]
pub struct RepetitionDetection {
    /// Text already sent to the frontend that must be removed from its tail.
    pub emitted_suffix: String,
    pub position: usize,
}

#[derive(Default)]
pub struct RepetitionGuard {
    pieces: Vec<String>,
}

impl RepetitionGuard {
    const MAX_WINDOW: usize = 48;
    const SINGLE_REPEAT_LIMIT: usize = 9;
    const NGRAM_REPEAT_LIMIT: usize = 4;

    pub fn observe(&mut self, piece: &str) -> Option<RepetitionDetection> {
        let normalized = normalize(piece);
        if normalized.is_empty() {
            return None;
        }

        self.pieces.push(piece.to_string());
        if self.pieces.len() > Self::MAX_WINDOW {
            self.pieces.remove(0);
        }

        let start = self.repeated_single_start().or_else(|| self.repeated_ngram_start())?;
        // The final piece triggered detection and has not been emitted yet.
        let emitted_suffix = self.pieces[start..self.pieces.len().saturating_sub(1)].concat();
        Some(RepetitionDetection {
            emitted_suffix,
            position: self.pieces.len(),
        })
    }

    fn repeated_single_start(&self) -> Option<usize> {
        let last = normalize(self.pieces.last()?);
        let mut count = 0;
        for piece in self.pieces.iter().rev() {
            if normalize(piece) == last {
                count += 1;
            } else {
                break;
            }
        }
        (count >= Self::SINGLE_REPEAT_LIMIT).then_some(self.pieces.len() - count)
    }

    fn repeated_ngram_start(&self) -> Option<usize> {
        for width in 2..=6 {
            if self.pieces.len() < width * Self::NGRAM_REPEAT_LIMIT {
                continue;
            }
            let pattern_start = self.pieces.len() - width;
            let pattern: Vec<String> = self.pieces[pattern_start..]
                .iter()
                .map(|piece| normalize(piece))
                .collect();
            // A one-token loop is deliberately handled by the less-sensitive
            // single-token threshold above, so legitimate repeated words and
            // list punctuation do not trip the n-gram circuit breaker early.
            if pattern.iter().any(String::is_empty) || pattern.windows(2).all(|pair| pair[0] == pair[1]) {
                continue;
            }
            let mut repeats = 1;
            while self.pieces.len() >= width * (repeats + 1) {
                let start = self.pieces.len() - width * (repeats + 1);
                let candidate = &self.pieces[start..start + width];
                if candidate.iter().map(|piece| normalize(piece)).eq(pattern.iter().cloned()) {
                    repeats += 1;
                } else {
                    break;
                }
            }
            if repeats >= Self::NGRAM_REPEAT_LIMIT {
                return Some(self.pieces.len() - repeats * width);
            }
        }
        None
    }
}

fn normalize(piece: &str) -> String {
    piece.trim().to_lowercase()
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
    fn catches_short_sequence_loop() {
        let mut guard = RepetitionGuard::default();
        for _ in 0..3 {
            assert!(guard.observe("A").is_none());
            assert!(guard.observe("B").is_none());
        }
        assert!(guard.observe("A").is_none());
        assert!(guard.observe("B").is_some());
    }
}
