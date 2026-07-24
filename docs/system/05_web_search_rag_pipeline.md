# 05. Adaptive Web Search & RAG Pipeline

## Overview

The **Adaptive Web Search & RAG Pipeline** (`src-tauri/src/web_search/`) is an advanced multi-source retrieval engine capable of parallel query execution across multiple search providers, deep web scraping via Crawl4AI, BM25 keyword reranking, vector semantic embeddings matching, and multi-pass confidence scoring.

---

## Pipeline Execution Architecture

```
                       ┌─────────────────────────┐
                       │  `search_web` Tool Call │
                       └────────────┬────────────┘
                                    │
                                    ▼
                      ┌───────────────────────────┐
                      │ Source Router             │
                      │ (`source_router.rs`)      │
                      └─────────────┬─────────────┘
                                    │
                                    ▼
                      ┌───────────────────────────┐
                      │ Multi-Worker Parallel Exec│
                      │ (`worker_runtime.rs`)     │
                      └─────────────┬─────────────┘
                                    │
         ┌──────────────┬───────────┼───────────┬──────────────┐
         ▼              ▼           ▼           ▼              ▼
   [DuckDuckGo]   [Brave Search] [SearXNG]  [Bing RSS]  [Crawl4AI Scraper]
         │              │           │           │              │
         └──────────────┴───────────┼───────────┴──────────────┘
                                    │
                                    ▼
                      ┌───────────────────────────┐
                      │ BM25 & Semantic Reranker  │
                      │ (`bm25.rs`)               │
                      └─────────────┬─────────────┘
                                    │
                                    ▼
                      ┌───────────────────────────┐
                      │ Confidence Scorer         │
                      │ (`orchestrator.rs`)       │
                      └─────────────┬─────────────┘
                                    │
                          Confidence < 0.55?
                         ┌──────────┴──────────┐
                        Yes                    No
                         │                     │
                         ▼                     ▼
             [ Query Expansion Pass ]     [ Grounded Prompt ]
             Secondary Search Workers     Synthesize Final Output
```

---

## Subsystem Details

### 1. Source Router (`source_router.rs`)
Classifies the raw query into intent categories:
- **Tech / Developer:** Prioritizes documentation, GitHub, arXiv.
- **News / World Events:** Prioritizes Bing RSS, news APIs.
- **Finance / Currency:** Prioritizes market ticker endpoints.
- **Weather:** Prioritizes meteorological APIs.
- **General Web:** Dispatches to DuckDuckGo, Brave, and SearXNG.

### 2. Parallel Worker Runtime (`worker_runtime.rs`)
Executes async HTTP queries concurrently across enabled search engines with configurable timeouts (8,000ms default) and automatic provider health circuit breakers (`health.rs`).

### 3. Deep Web Scraper (`crawl4ai.rs`)
For complex technical queries requiring full web page context, Crawl4AI extracts clean Markdown text from target URLs while stripping scripts, ads, and navigation boilerplate.

### 4. BM25 & Semantic Reranker (`bm25.rs`)
- **Keyword BM25 Scoring:** Computes TF-IDF term frequency scores across all retrieved web page snippets.
- **Vector Cosine Similarity:** If an embedding endpoint is available, generates vector embeddings for candidate snippets and computes cosine similarity against the query vector.
- **Combined Score:** Merges BM25 and vector similarity into a unified relevance score.

### 5. Multi-Pass Confidence Scorer (`orchestrator.rs`)
Calculates a overall grounding confidence score:
$$\text{Confidence} = \text{Relevance} \times \text{Agreement} \times \text{Coverage}$$
If $\text{Confidence} < 0.55$, the orchestrator triggers an automatic secondary query expansion pass to gather additional supporting web evidence before grounding the final LLM prompt.
