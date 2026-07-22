use super::{EvidenceChunk, SearchResult};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RankedChunk {
    pub result_index: usize,
    pub content: String,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct RankedEvidence {
    pub chunk: EvidenceChunk,
    pub score: f32,
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
    let ranked = chunks
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

    // Never turn a failed lexical match into source context. A zero-score
    // excerpt is not evidence and was allowing label-only Wikidata matches to
    // steer conversational replies.
    if ranked.is_empty() {
        return Vec::new();
    }
    select_ranked(ranked, max_chars)
}

/// Cross-lingual semantic ranking. This is the primary path when Tier 0 is
/// available, so languages without whitespace boundaries (and dialect/code
/// mixing) do not depend on English-style lexical tokenization.
pub fn rank_with_embeddings(
    endpoint: &str,
    query: &str,
    documents: &[SearchResult],
    max_chars: usize,
) -> Result<Vec<RankedChunk>, String> {
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
        return Ok(Vec::new());
    }
    let mut inputs = Vec::with_capacity(chunks.len() + 1);
    inputs.push(query.chars().take(384).collect::<String>());
    inputs.extend(
        chunks
            .iter()
            .map(|(_, content)| content.chars().take(384).collect::<String>()),
    );
    let mut vectors = crate::engine::embedding_runtime::embed_retrieval(
        endpoint,
        &inputs[0],
        &inputs[1..],
    )?;
    let query_vector = vectors.remove(0);
    let ranked = chunks
        .into_iter()
        .zip(vectors)
        .filter_map(|((result_index, content), vector)| {
            let score = crate::engine::embedding_runtime::cosine_similarity(&query_vector, &vector);
            (score >= 0.30).then_some(RankedChunk {
                result_index,
                content,
                score: score as f64,
            })
        })
        .collect::<Vec<_>>();
    Ok(select_ranked(ranked, max_chars))
}

fn select_ranked(mut ranked: Vec<RankedChunk>, max_chars: usize) -> Vec<RankedChunk> {
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
            if selected.len() >= 10 {
                return selected;
            }
        }
    }
    selected
}

pub fn semantic_rerank_top_score(
    embedding_endpoint: Option<&str>,
    query: &str,
    chunks: &[EvidenceChunk],
) -> f32 {
    if chunks.is_empty() {
        return 0.0;
    }
    if let Some(endpoint) = embedding_endpoint {
        let mut inputs = Vec::with_capacity(chunks.len() + 1);
        inputs.push(query.chars().take(384).collect::<String>());
        inputs.extend(
            chunks
                .iter()
                .map(|chunk| chunk.text.chars().take(384).collect::<String>()),
        );
        if let Ok(mut vectors) = crate::engine::embedding_runtime::embed_retrieval(
            endpoint,
            &inputs[0],
            &inputs[1..],
        ) {
            if !vectors.is_empty() {
                let query_vector = vectors.remove(0);
                return vectors
                    .iter()
                    .map(|vector| {
                        crate::engine::embedding_runtime::cosine_similarity(&query_vector, vector)
                    })
                    .fold(0.0_f32, f32::max)
                    .clamp(0.0, 1.0);
            }
        }
    }
    let query_terms = terms(query);
    if query_terms.is_empty() {
        return 0.5;
    }

    let mut best_score = 0.0f32;
    for chunk in chunks {
        let chunk_terms = terms(&chunk.text);
        let matches = query_terms
            .iter()
            .filter(|t| chunk_terms.contains(*t))
            .count();
        let score = (matches as f32) / (query_terms.len() as f32);
        if score > best_score {
            best_score = score;
        }
    }
    best_score.clamp(0.0, 1.0)
}

/// Scores heterogeneous API/RSS/web evidence together so worker completion
/// order cannot make a fast but irrelevant source dominate the final prompt.
/// The caller owns the acceptance threshold because current-news questions
/// need a stricter relevance floor than broad background research.
pub fn rerank_evidence(
    endpoint: &str,
    query: &str,
    chunks: Vec<EvidenceChunk>,
) -> Result<Vec<RankedEvidence>, String> {
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let mut inputs = Vec::with_capacity(chunks.len() + 1);
    inputs.push(query.chars().take(384).collect::<String>());
    inputs.extend(
        chunks
            .iter()
            .map(|chunk| chunk.text.chars().take(384).collect::<String>()),
    );
    let mut vectors = crate::engine::embedding_runtime::embed_retrieval(
        endpoint,
        &inputs[0],
        &inputs[1..],
    )?;
    let query_vector = vectors.remove(0);
    let mut ranked = chunks
        .into_iter()
        .zip(vectors)
        .map(|(chunk, vector)| {
            let score = crate::engine::embedding_runtime::cosine_similarity(&query_vector, &vector);
            RankedEvidence { chunk, score }
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(10);
    Ok(ranked)
}

pub fn corroboration_score(chunks: &[EvidenceChunk]) -> f32 {
    if chunks.len() < 2 {
        return 0.5;
    }

    let mut total_sim = 0.0;
    let mut pairs = 0;

    for i in 0..chunks.len() {
        for j in i + 1..chunks.len() {
            if chunks[i].host != chunks[j].host {
                let terms_i = terms(&chunks[i].text);
                let terms_j = terms(&chunks[j].text);
                if !terms_i.is_empty() && !terms_j.is_empty() {
                    let common = terms_i.intersection(&terms_j).count() as f32;
                    let union = terms_i.union(&terms_j).count() as f32;
                    if union > 0.0 {
                        total_sim += common / union;
                        pairs += 1;
                    }
                }
            }
        }
    }

    if pairs == 0 {
        0.5
    } else {
        (total_sim / pairs as f32).clamp(0.0, 1.0)
    }
}

pub fn key_term_coverage(query: &str, chunks: &[EvidenceChunk]) -> f32 {
    let q_terms = terms(query);
    if q_terms.is_empty() {
        return 1.0;
    }

    let mut covered = HashSet::new();
    for chunk in chunks {
        let chunk_terms = terms(&chunk.text);
        for term in &q_terms {
            if chunk_terms.contains(term) {
                covered.insert(term.clone());
            }
        }
    }

    (covered.len() as f32 / q_terms.len() as f32).clamp(0.0, 1.0)
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
    use super::*;
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

    #[test]
    fn calculates_key_term_coverage() {
        let chunks = vec![EvidenceChunk {
            text: "Rust compiler ownership memory safety".to_string(),
            source_url: "https://example.com".to_string(),
            source_title: "Rust".to_string(),
            host: "example.com".to_string(),
        }];
        let score = key_term_coverage("Rust compiler safety", &chunks);
        assert!(score > 0.9);
    }

    #[test]
    fn rejects_documents_without_any_relevant_terms() {
        let documents = vec![SearchResult {
            title: "Entity match".to_string(),
            url: "https://example.com/entity".to_string(),
            snippet: String::new(),
            content: "A catalogue record with unrelated metadata.".to_string(),
        }];
        assert!(rank("today's breaking news", &documents, 500).is_empty());
    }
}
