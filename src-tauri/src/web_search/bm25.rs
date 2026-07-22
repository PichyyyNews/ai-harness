use super::SearchResult;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RankedChunk {
    pub result_index: usize,
    pub content: String,
    score: f64,
}

pub fn rank(query: &str, documents: &[SearchResult], max_chars: usize) -> Vec<RankedChunk> {
    let terms = terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let chunks = documents
        .iter()
        .enumerate()
        .flat_map(|(index, document)| {
            split_chunks(&document.content)
                .into_iter()
                .map(move |content| (index, content))
        })
        .collect::<Vec<_>>();
    if chunks.is_empty() {
        return Vec::new();
    }
    let average_length = chunks
        .iter()
        .map(|(_, content)| content.split_whitespace().count() as f64)
        .sum::<f64>()
        / chunks.len() as f64;
    let document_frequency = terms
        .iter()
        .map(|term| {
            let count = chunks
                .iter()
                .filter(|(_, content)| normalized(content).contains(term))
                .count() as f64;
            (term, count)
        })
        .collect::<HashMap<_, _>>();
    let total = chunks.len() as f64;
    let mut ranked = chunks
        .into_iter()
        .map(|(result_index, content)| {
            let length = content.split_whitespace().count() as f64;
            let haystack = normalized(&content);
            let score = terms
                .iter()
                .map(|term| {
                    let frequency = haystack.matches(term).count() as f64;
                    if frequency == 0.0 {
                        return 0.0;
                    }
                    let df = *document_frequency.get(term).unwrap_or(&0.0);
                    let idf = ((total - df + 0.5) / (df + 0.5) + 1.0).ln();
                    let k1 = 1.2;
                    let b = 0.75;
                    idf * (frequency * (k1 + 1.0))
                        / (frequency + k1 * (1.0 - b + b * length / average_length.max(1.0)))
                })
                .sum();
            RankedChunk {
                result_index,
                content,
                score,
            }
        })
        .filter(|chunk| chunk.score >= 0.20)
        .collect::<Vec<_>>();
    // Some languages do not use whitespace between words. If BM25 cannot find
    // a reliable token match, preserve a compact, diverse set of first-party
    // excerpts instead of silently reducing grounding to a single result.
    if ranked.is_empty() {
        ranked = documents
            .iter()
            .enumerate()
            .filter_map(|(result_index, document)| {
                split_chunks(&document.content)
                    .into_iter()
                    .next()
                    .map(|content| RankedChunk {
                        result_index,
                        content,
                        score: 0.0,
                    })
            })
            .collect();
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut selected = Vec::new();
    let mut used = 0;
    let mut represented_documents = HashSet::new();
    let mut selected_ranked_indexes = HashSet::new();
    // First pass gives the model corroboration across sources. A second pass
    // adds depth only after each useful source has had a chance to contribute.
    for require_new_source in [true, false] {
        for (ranked_index, chunk) in ranked.iter().enumerate() {
            if require_new_source && represented_documents.contains(&chunk.result_index) {
                continue;
            }
            if selected_ranked_indexes.contains(&ranked_index) {
                continue;
            }
            if used + chunk.content.len() > max_chars {
                continue;
            }
            used += chunk.content.len();
            represented_documents.insert(chunk.result_index);
            selected_ranked_indexes.insert(ranked_index);
            selected.push(chunk.clone());
            if selected.len() >= 6 {
                return selected;
            }
        }
    }
    selected
}

fn split_chunks(content: &str) -> Vec<String> {
    let words = content.split_whitespace().collect::<Vec<_>>();
    words.chunks(220).map(|chunk| chunk.join(" ")).collect()
}

fn terms(value: &str) -> HashSet<String> {
    normalized(value)
        .split_whitespace()
        .filter(|term| term.len() >= 3)
        .map(ToOwned::to_owned)
        .collect()
}

fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::rank;
    use crate::web_search::SearchResult;

    #[test]
    fn ranks_relevant_content_inside_the_budget() {
        let documents = vec![
            SearchResult {
                title: "Rust".to_string(),
                url: "https://example.com/rust".to_string(),
                snippet: String::new(),
                content: "Rust release notes discuss ownership and the compiler.".to_string(),
            },
            SearchResult {
                title: "Other".to_string(),
                url: "https://example.com/other".to_string(),
                snippet: String::new(),
                content: "Gardening advice for spring flowers.".to_string(),
            },
        ];
        let selected = rank("latest Rust compiler release", &documents, 500);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].result_index, 0);
    }
}
