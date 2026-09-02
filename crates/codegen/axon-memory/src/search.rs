//! Hybrid search combining FTS5 BM25 + sqlite-vec KNN + temporal decay + source weighting + MMR.
//!
//! The search pipeline:
//! 1. FTS5 keyword search (always available)
//! 2. Vector KNN search (when sqlite-vec + embeddings are available)
//! 3. Merge results by chunk_id, normalize scores to [0,1]
//! 4. Skip content-free chunks: empty/boilerplate templates (the
//!    auto-generated `MEMORY.md` stub) never appear in results / injection
//! 5. Apply temporal decay: evergreen sources (global, workspace) are exempt;
//!    session chunks decay with exponential half-life:
//!    `decayed = base × e^(-λ × age_days)` where `λ = ln(2) / half_life_days`
//! 6. Apply source weights + access-frequency boost, filter by `min_score`,
//!    rank on the unclamped score, then clamp the stored display score to [0,1]
//! 7. MMR diversity re-ranking (opt-in, penalizes redundant results)
//! 8. Limit to `max_results`
//!
//! Graceful degradation: if vector search is unavailable, falls back to FTS-only
//! with `text_weight = 1.0`.

use std::collections::HashMap;

use super::embedding::EmbeddingProvider;
use super::index::MemoryIndex;
use axon_config_types::{MemorySearchConfig, SearchFusion};

/// A search result with merged scoring from FTS and vector search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: f64,
    pub snippet: String,
    pub source: String,
    pub created_at: i64,
}

/// Returns `true` for sources that contain curated long-term knowledge
/// and should not be penalized by temporal decay.
///
/// Evergreen: `"global"` (MEMORY.md), `"workspace"` (project MEMORY.md).
/// Decaying: `"session"` (auto-generated session logs).
fn is_evergreen_source(source: &str) -> bool {
    matches!(source, "global" | "workspace")
}

/// Returns `true` when a chunk is an empty/boilerplate template (e.g. the
/// auto-generated `MEMORY.md` stub) and should be filtered out.
///
/// True when the chunk is structurally empty, or matches a known scaffold
/// template via [`super::dream::is_scaffold_template`]. The marker branch is
/// scoped to evergreen sources, where scaffold templates live, so a session
/// chunk that merely quotes a marker phrase is kept.
fn is_content_free(text: &str, source: &str) -> bool {
    is_structurally_empty(text)
        || (is_evergreen_source(source) && super::dream::is_scaffold_template(text))
}

/// Returns `true` when `text` has no substantive content after stripping ATX
/// headings, HTML comments, and whitespace. Blockquotes are NOT stripped —
/// they are real user content.
fn is_structurally_empty(text: &str) -> bool {
    // Fast path: no comment marker means no multi-line span to strip.
    if !text.contains("<!--") {
        return lines_are_scaffolding(text);
    }

    // Strip HTML comments (which may span lines) before the line scan.
    let mut without_comments = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        match rest[start + "<!--".len()..].find("-->") {
            Some(end) => {
                without_comments.push_str(&rest[..start]);
                let after = start + "<!--".len() + end + "-->".len();
                rest = &rest[after..];
            }
            None => {
                // Unterminated comment: keep the remainder as literal text so a
                // comment split across a chunk boundary can't drop real content.
                without_comments.push_str(rest);
                rest = "";
                break;
            }
        }
    }
    without_comments.push_str(rest);

    lines_are_scaffolding(&without_comments)
}

/// Returns `true` when every non-blank line is an ATX heading (per
/// [`super::chunker::header_level`]). Any other non-blank line is content.
fn lines_are_scaffolding(text: &str) -> bool {
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if super::chunker::header_level(line).is_some() {
            continue;
        }
        return false;
    }
    true
}

/// Compute the temporal decay multiplier for a chunk.
///
/// - Evergreen sources → `1.0` (no decay).
/// - Session sources → exponential decay: `e^(-λ × age_days)` where
///   `λ = ln(2) / half_life_days`. Score halves every `half_life_days`.
/// - `half_life = None` → decay disabled, returns `1.0` for all sources.
fn temporal_decay_multiplier(
    source: &str,
    created_at: i64,
    now_secs: i64,
    half_life_days: Option<f64>,
) -> f64 {
    let Some(half_life) = half_life_days else {
        return 1.0;
    };
    if is_evergreen_source(source) {
        return 1.0;
    }
    if half_life <= 0.0 {
        return 1.0;
    }
    // No upper clamp on age: with exponential decay a 2-year-old chunk at
    // 30-day half-life scores ~6e-8, well below any reasonable min_score.
    let age_days = ((now_secs - created_at.max(0)) as f64 / 86400.0).max(0.0);
    let lambda = f64::ln(2.0) / half_life;
    (-lambda * age_days).exp()
}

/// Run a hybrid search across the memory index.
///
/// Combines FTS5 keyword search with optional vector KNN similarity.
/// Falls back to FTS-only when vector search is unavailable.
///
/// Structured so that `&MemoryIndex` is never held across `.await` points,
/// allowing the caller's future to be `Send` even though `MemoryIndex` is `!Sync`.
#[tracing::instrument(name = "memory.hybrid_search", skip_all, fields(
    max_results = config.max_results,
))]
/// Reciprocal Rank Fusion over the FTS and vector result lists, with source
/// weights applied **in rank space**.
///
/// `RRF(d) = Σ_lists  list_weight / (k + rank_d / source_weight_d)`, 1-based
/// ranks, lists the chunk is absent from contributing nothing.
///
/// Why this exists alongside the weighted path: RRF reads only the ORDER of
/// each list, never the scores. That removes three scale hazards the weighted
/// path defends against by hand — BM25 is min-max normalized *within* the
/// result set (so its worst hit is always exactly 0.0), cosine is on an
/// absolute scale, and the product of base × decay × source_weight × access
/// boost is compared against an ABSOLUTE `min_score`, which once capped global
/// chunks at `text_weight × source_weight = 0.21` and hid them entirely.
///
/// **Why the source weight divides the RANK and is not a multiplier.** The
/// first cut of this applied `source_weight` downstream, as a factor on the
/// fused score, and it was much worse than the weighted path: measured 0/12
/// queries returning a global chunk against 9/12. With k=60, `1/(k + rank)`
/// barely varies — the whole candidate set lands in a ~0.07 band — so a 0.7
/// factor applied afterwards dwarfs every rank difference and becomes the
/// dominant ranking signal. Multiplying the *contribution* instead
/// (`w / (k + rank)`) has the same defect for the same reason.
///
/// Dividing the rank makes the weight commensurate with what RRF actually
/// measures. `w = 0.7` at rank 2 competes as though it were rank 2.86: a
/// demotion of a fraction of a place, which is what "this source is somewhat
/// less trusted" should mean, rather than a 30% score cut applied to items
/// that differ from each other by 1.6%.
///
/// Because the weight is consumed here, the caller must NOT apply
/// `source_weight` again downstream.
///
/// Results are renormalized so the best hit is 1.0, keeping the [0,1] contract
/// the display score and `min_score` gate expect — at the cost of making
/// `min_score` relative to the best hit rather than absolute.
fn fuse_rrf(
    index: &MemoryIndex,
    fts_results: &[super::index::FtsResult],
    vec_results: &[(String, f32)],
    config: &MemorySearchConfig,
) -> HashMap<String, f64> {
    // Guard the rank constant: k <= -1 would divide by zero at rank 1, and a
    // negative k inverts the ordering. Clamp rather than reject so a bad
    // config degrades to the paper default instead of returning nothing.
    let k = if config.rrf_k.is_finite() && config.rrf_k >= 0.0 {
        config.rrf_k as f64
    } else {
        60.0
    };
    let text_weight = config.text_weight as f64;
    let vector_weight = config.vector_weight as f64;

    let mut fused: HashMap<String, f64> = HashMap::new();

    // Source weight for a chunk. An id with no row — a chunk that vanished
    // between the list query and now — takes the neutral 1.0 rather than
    // being dropped here; the downstream loop drops it anyway when its own
    // `get_chunk` misses, and that is the one place it should happen.
    let weight_for = |chunk_id: &str| -> f64 {
        index
            .get_chunk(chunk_id)
            .ok()
            .flatten()
            .and_then(|c| config.source_weights.get(&c.source).copied())
            .unwrap_or(1.0) as f64
    };

    // Both input lists arrive already ordered best-first, so position IS rank.
    let contribute =
        |chunk_id: &str, rank: f64, list_weight: f64, fused: &mut HashMap<String, f64>| {
            let w = weight_for(chunk_id);
            // A non-positive weight means the source is switched off. Effective
            // rank would be infinite, so contribute nothing rather than dividing
            // by zero or flipping the sign.
            if w <= 0.0 {
                return;
            }
            let effective_rank = rank / w;
            *fused.entry(chunk_id.to_string()).or_insert(0.0) += list_weight / (k + effective_rank);
        };

    for (i, r) in fts_results.iter().enumerate() {
        contribute(&r.chunk_id, (i + 1) as f64, text_weight, &mut fused);
    }
    for (i, (chunk_id, _distance)) in vec_results.iter().enumerate() {
        contribute(chunk_id, (i + 1) as f64, vector_weight, &mut fused);
    }

    // Renormalize to [0,1]. `max` is over a non-empty map by construction here,
    // but fold defensively so an empty input yields an empty result rather
    // than a division by zero.
    let max = fused.values().copied().fold(0.0_f64, f64::max);
    if max > 0.0 {
        for score in fused.values_mut() {
            *score /= max;
        }
    }
    fused
}

pub async fn hybrid_search(
    index: &MemoryIndex,
    embedding_provider: Option<&dyn EmbeddingProvider>,
    query: &str,
    config: &MemorySearchConfig,
) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
    let candidate_limit = config.max_results * 3;

    // Phase 1 (sync): FTS search + supplemental evergreen query so
    // global/workspace chunks aren't crowded out by session volume.
    let mut fts_results = index.search_fts(query, candidate_limit).unwrap_or_default();
    let evergreen = index
        .search_fts_by_sources(query, candidate_limit, &["global", "workspace"])
        .unwrap_or_default();
    let existing: std::collections::HashSet<String> =
        fts_results.iter().map(|r| r.chunk_id.clone()).collect();
    for r in evergreen {
        if !existing.contains(&r.chunk_id) {
            fts_results.push(r);
        }
    }
    let vec_available = index.vec_available();

    // Phase 2 (async): embed query — no &index borrow here
    let query_embedding = if vec_available {
        if let Some(provider) = embedding_provider {
            match provider.embed_batch(&[query]).await {
                Ok(embeddings) if !embeddings.is_empty() => {
                    Some(embeddings.into_iter().next().unwrap())
                }
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(error = %e, "embedding query failed, falling back to FTS-only");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Phase 3 (sync): vector search + scoring + merge
    hybrid_search_merge(index, fts_results, query_embedding.as_deref(), config)
}

/// Synchronous merge phase: vector search (if embedding provided), score
/// normalization, temporal decay, source weighting, MMR, and truncation.
pub(super) fn hybrid_search_merge(
    index: &MemoryIndex,
    fts_results: Vec<super::index::FtsResult>,
    query_embedding: Option<&[f32]>,
    config: &MemorySearchConfig,
) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
    let candidate_limit = config.max_results * 3;

    let vec_results = if let Some(embedding) = query_embedding {
        index
            .vector_search(embedding, candidate_limit)
            .unwrap_or_default()
    } else {
        vec![]
    };

    // Normalize and merge scores.
    //
    // Per-chunk scoring strategy:
    //   - Chunks with BOTH FTS and vector matches: weighted combination
    //     (text_weight × fts_score + vector_weight × vec_score)
    //   - Chunks with ONLY FTS matches: score = fts_score (full weight, not
    //     penalized to text_weight just because other chunks have vectors)
    //   - Chunks with ONLY vector matches: score = vector_weight × vec_score
    //
    // This ensures FTS-only chunks (e.g., global MEMORY.md with no embedding
    // match) can still score high enough to pass min_score.
    let mut fts_scores: HashMap<String, f64> = HashMap::new();
    let mut vec_scores: HashMap<String, f64> = HashMap::new();

    // Normalize FTS BM25 scores to [0,1] (BM25 scores are negative in FTS5,
    // more negative = better match).
    //
    // Two transforms. The min-max one is relative to the result set and is the
    // historical default; the saturating one is absolute. See
    // `bm25_saturation` for why the difference matters — in short, min-max
    // hands the best keyword hit exactly 1.0 however irrelevant it is, which
    // no vector-only chunk can outrank.
    if !fts_results.is_empty() {
        let knee = config.bm25_saturation as f64;
        if knee > 0.0 {
            for r in &fts_results {
                // `m` is the BM25 magnitude: 0 at no signal, growing with
                // rarer terms and more of them matched. `m / (m + knee)` is
                // monotonic into [0,1) and depends on nothing but this chunk.
                let m = (-r.rank).max(0.0);
                fts_scores.insert(r.chunk_id.clone(), m / (m + knee));
            }
        } else {
            let min_rank = fts_results
                .iter()
                .map(|r| r.rank)
                .fold(f64::INFINITY, f64::min);
            let max_rank = fts_results
                .iter()
                .map(|r| r.rank)
                .fold(f64::NEG_INFINITY, f64::max);
            // When there's only 1 FTS result, min_rank == max_rank, so range = EPSILON
            // and normalized = 1.0. This is correct: a single result gets full score.
            let range = (max_rank - min_rank).max(f64::EPSILON);

            for r in &fts_results {
                // FTS5 rank: more negative = better. Normalize so best = 1.0
                let normalized = 1.0 - (r.rank - min_rank) / range;
                fts_scores.insert(r.chunk_id.clone(), normalized);
            }
        }
    }

    // Normalize vector distances to [0,1] similarity using absolute scale.
    //
    // For normalized embeddings, L2 distance ranges from 0 (identical) to 2
    // (opposite). Using `similarity = 1.0 - distance / 2.0` maps this to
    // [0, 1] on an absolute scale, avoiding the compression problem where
    // relative normalization (`1 - d/max_d`) collapses all scores to near-zero
    // when candidates cluster in a narrow distance band (common for
    // high-dimensional embeddings due to concentration of measure).
    //
    // The constant `2.0` is the theoretical maximum L2 distance between two
    // unit-norm vectors: ||u - v||₂ = sqrt(2 - 2·cos(θ)) ≤ sqrt(4) = 2.
    const MAX_L2_DISTANCE: f64 = 2.0;
    for (chunk_id, distance) in &vec_results {
        let similarity = (1.0 - (*distance as f64 / MAX_L2_DISTANCE)).clamp(0.0, 1.0);
        vec_scores.insert(chunk_id.clone(), similarity);
    }

    // Merge per-chunk scores. Everything downstream — temporal decay, source
    // weights, the access boost, the `min_score` gate and the clamped display
    // score — is identical for both strategies; they differ only in how the
    // two candidate lists become one base score per chunk.
    let scores: HashMap<String, f64> = match config.fusion {
        SearchFusion::Rrf => fuse_rrf(index, &fts_results, &vec_results, config),
        SearchFusion::Weighted => {
            // use max(fts_only, hybrid) so FTS-only chunks are never penalized
            // by the existence of unrelated vector results.
            let mut scores: HashMap<String, f64> = HashMap::new();
            let text_weight = config.text_weight as f64;
            let vector_weight = config.vector_weight as f64;

            // Collect all unique chunk IDs across both result sets.
            let all_chunk_ids: std::collections::HashSet<&String> =
                fts_scores.keys().chain(vec_scores.keys()).collect();

            for chunk_id in all_chunk_ids {
                let fts = fts_scores.get(chunk_id).copied().unwrap_or(0.0);
                let vec = vec_scores.get(chunk_id).copied().unwrap_or(0.0);

                let score = if fts > 0.0 && vec > 0.0 {
                    // Both signals available: weighted combination, but never worse
                    // than the FTS score alone (since text_weight < 1.0 would otherwise
                    // penalize a strong keyword match).
                    let hybrid = text_weight * fts + vector_weight * vec;
                    hybrid.max(fts)
                } else if fts > 0.0 {
                    // FTS-only: full FTS score (not penalized to text_weight)
                    fts
                } else {
                    // Vector-only: weighted vector score
                    vector_weight * vec
                };

                scores.insert(chunk_id.clone(), score);
            }
            scores
        }
    };

    // Apply temporal decay and source weights, build results
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let half_life = config.effective_half_life_days();

    // Pairs the unclamped ranking score with each result (see the raw_score /
    // display_score split below).
    let mut ranked: Vec<(f64, SearchResult)> = Vec::new();

    for (chunk_id, base_score) in &scores {
        let Some(chunk) = index.get_chunk(chunk_id).ok().flatten() else {
            continue;
        };

        // Filter at search time (not index time) so already-indexed stubs are
        // excluded without requiring a reindex.
        if is_content_free(&chunk.text, &chunk.source) {
            continue;
        }

        let decay_multiplier =
            temporal_decay_multiplier(&chunk.source, chunk.created_at, now_secs, half_life);

        // RRF already consumed the source weight, in rank space, inside
        // `fuse_rrf`. Applying it again here would both double-count it and
        // reintroduce exactly the defect that made the first prototype worse
        // than the weighted path: a multiplier on scores that RRF has
        // compressed into a narrow band swamps every rank difference.
        let source_weight = match config.fusion {
            SearchFusion::Rrf => 1.0,
            SearchFusion::Weighted => config
                .source_weights
                .get(&chunk.source)
                .copied()
                .unwrap_or(1.0) as f64,
        };

        // Access-frequency boost: chunks retrieved before score slightly higher.
        //
        // Uses ln(1 + access_count) so:
        // - 0 accesses → boost = 1.0 (no penalty)
        // - 1 access   → boost ≈ 1.035
        // - 10 accesses → boost ≈ 1.120
        // - 100 accesses → boost ≈ 1.230
        //
        // The 0.05 scale factor keeps the boost modest so retrieval relevance
        // (BM25 / vector similarity) remains the primary ranking signal.
        let access_boost = 1.0 + (chunk.access_count as f64).ln_1p() * 0.05;
        // access_boost is an unbounded multiplier (> 1.0), so the product can
        // exceed 1.0 for top evergreen chunks. Rank on the unclamped raw_score
        // (so the boost still orders chunks that would otherwise both clamp to
        // 1.0), but store the clamped display_score so it reads as a [0,1]
        // similarity. Gating on display_score keeps the threshold and the
        // stored value in agreement.
        let raw_score = base_score * decay_multiplier * source_weight * access_boost;
        let display_score = raw_score.clamp(0.0, 1.0);

        if display_score >= config.min_score as f64 {
            ranked.push((
                raw_score,
                SearchResult {
                    chunk_id: chunk_id.clone(),
                    path: chunk.path.clone(),
                    start_line: chunk.start_line,
                    end_line: chunk.end_line,
                    score: display_score,
                    snippet: chunk.text.clone(),
                    source: chunk.source.clone(),
                    created_at: chunk.created_at,
                },
            ));
        }
    }

    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Split into aligned `relevance` (unclamped) + `results` (clamped) vectors
    // so MMR can rank on relevance. Only build `relevance` when MMR is enabled;
    // otherwise `mmr_rerank` early-returns before reading it.
    let mmr_enabled = config.mmr.enabled;
    let mut relevance: Vec<f64> = if mmr_enabled {
        Vec::with_capacity(ranked.len())
    } else {
        Vec::new()
    };
    let mut results: Vec<SearchResult> = Vec::with_capacity(ranked.len());
    for (raw_score, result) in ranked {
        if mmr_enabled {
            relevance.push(raw_score);
        }
        results.push(result);
    }

    // MMR diversity re-ranking (opt-in, applied before truncation)
    super::mmr::mmr_rerank(&mut results, &relevance, &config.mmr);

    results.truncate(config.max_results);

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::MockEmbeddingProvider;
    use crate::index::{MemoryIndex, init_sqlite_vec};
    use crate::storage::MemoryStorage;
    use axon_config_types::{MemoryIndexConfig, MemorySearchConfig};
    use tempfile::TempDir;

    fn test_index(tmp: &TempDir) -> MemoryIndex {
        init_sqlite_vec();
        let global = tmp.path().join("memory");
        let workspace = global.join("test_ws");
        let storage = MemoryStorage::with_paths(global, workspace);
        let db_path = tmp.path().join("test.sqlite");
        MemoryIndex::open_or_create(&db_path, storage, MemoryIndexConfig::default(), 4).unwrap()
    }

    #[tokio::test]
    async fn test_hybrid_search_fts_only() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let file_path = tmp.path().join("test.md");
        std::fs::write(&file_path, "# Guide\n\nRust programming language tutorial.").unwrap();
        idx.reindex_file(&file_path, "workspace").unwrap();

        let config = MemorySearchConfig::default();
        let results = hybrid_search(&idx, None, "rust programming", &config)
            .await
            .unwrap();

        assert!(!results.is_empty(), "should find results via FTS");
        assert!(results[0].snippet.contains("Rust"));
        assert!(
            results[0].created_at > 0,
            "created_at must propagate from ChunkRecord (got {})",
            results[0].created_at,
        );
    }

    #[tokio::test]
    async fn test_hybrid_search_no_match() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let file_path = tmp.path().join("test.md");
        std::fs::write(&file_path, "# Guide\n\nPython tutorial.").unwrap();
        idx.reindex_file(&file_path, "workspace").unwrap();

        let config = MemorySearchConfig::default();
        let results = hybrid_search(&idx, None, "haskell monads", &config)
            .await
            .unwrap();

        assert!(results.is_empty(), "should not find unrelated content");
    }

    #[tokio::test]
    async fn test_hybrid_search_respects_max_results() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        // Create multiple matching files
        for i in 0..10 {
            let file_path = tmp.path().join(format!("test_{i}.md"));
            std::fs::write(&file_path, format!("# Doc {i}\n\nRust content {i}.")).unwrap();
            idx.reindex_file(&file_path, "workspace").unwrap();
        }

        let config = MemorySearchConfig {
            max_results: 3,
            min_score: 0.0, // accept all
            ..Default::default()
        };

        let results = hybrid_search(&idx, None, "rust content", &config)
            .await
            .unwrap();

        assert!(
            results.len() <= 3,
            "should respect max_results, got {}",
            results.len()
        );
    }

    #[tokio::test]
    async fn test_hybrid_search_empty_index() {
        let tmp = TempDir::new().unwrap();
        let idx = test_index(&tmp);

        let config = MemorySearchConfig::default();
        let results = hybrid_search(&idx, None, "anything", &config)
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_hybrid_search_source_weights() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let ws_file = tmp.path().join("ws.md");
        std::fs::write(&ws_file, "# WS\n\nRust workspace content.").unwrap();
        idx.reindex_file(&ws_file, "workspace").unwrap();

        let gl_file = tmp.path().join("gl.md");
        std::fs::write(&gl_file, "# GL\n\nRust global content.").unwrap();
        idx.reindex_file(&gl_file, "global").unwrap();

        let config = MemorySearchConfig {
            min_score: 0.0,
            ..Default::default()
        };

        let results = hybrid_search(&idx, None, "rust content", &config)
            .await
            .unwrap();

        // Both should be found; workspace should score higher due to source_weight
        if results.len() >= 2 {
            let ws_result = results.iter().find(|r| r.source == "workspace");
            let gl_result = results.iter().find(|r| r.source == "global");
            if let (Some(ws), Some(gl)) = (ws_result, gl_result) {
                assert!(
                    (ws.score - gl.score).abs() < 0.01,
                    "workspace and global should score equally (both weight=1.0)"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_hybrid_search_with_vector_and_fts() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);
        let mock = MockEmbeddingProvider { dimensions: 4 };

        // Index a file and embed its chunks
        let file_path = tmp.path().join("test.md");
        std::fs::write(&file_path, "# Guide\n\nRust programming language tutorial.").unwrap();
        idx.reindex_file(&file_path, "workspace").unwrap();

        // Embed the chunk
        let path_str = file_path.to_string_lossy().to_string();
        let chunk_id = format!("{path_str}:0");
        let chunk = idx.get_chunk(&chunk_id).unwrap().unwrap();
        let embeddings = mock.embed_batch(&[&chunk.text]).await.unwrap();
        idx.upsert_embedding(&chunk_id, &embeddings[0]).unwrap();

        // Search — should use both FTS and vector paths
        let config = MemorySearchConfig {
            min_score: 0.0,
            ..Default::default()
        };

        let results = hybrid_search(
            &idx,
            Some(&mock as &dyn EmbeddingProvider),
            "rust programming",
            &config,
        )
        .await
        .unwrap();

        assert!(!results.is_empty(), "hybrid search should find results");
        assert!(results[0].snippet.contains("Rust"));
        // With both FTS and vector results, score should combine both weights
        assert!(results[0].score > 0.0, "score should be positive");
    }

    // -----------------------------------------------------------------------
    // Temporal decay unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_evergreen_source() {
        assert!(is_evergreen_source("global"));
        assert!(is_evergreen_source("workspace"));
        assert!(!is_evergreen_source("session"));
        assert!(!is_evergreen_source("unknown"));
        assert!(!is_evergreen_source(""));
    }

    #[test]
    fn test_decay_disabled_returns_one() {
        assert_eq!(
            temporal_decay_multiplier("session", 0, 86400 * 90, None),
            1.0
        );
        assert_eq!(
            temporal_decay_multiplier("global", 0, 86400 * 90, None),
            1.0
        );
    }

    #[test]
    fn test_evergreen_sources_never_decay() {
        let now = 86400 * 365; // 1 year
        let created = 0; // created at epoch
        let half_life = Some(30.0);

        assert_eq!(
            temporal_decay_multiplier("global", created, now, half_life),
            1.0
        );
        assert_eq!(
            temporal_decay_multiplier("workspace", created, now, half_life),
            1.0
        );
    }

    #[test]
    fn test_session_chunks_decay_with_half_life() {
        let half_life = Some(30.0);
        let now = 86400 * 30; // 30 days after epoch
        let created = 0;

        let multiplier = temporal_decay_multiplier("session", created, now, half_life);
        // After exactly one half-life, multiplier should be ~0.5
        assert!(
            (multiplier - 0.5).abs() < 0.01,
            "30-day-old session chunk with 30-day half-life should score ~0.5, got {multiplier}"
        );
    }

    #[test]
    fn test_decay_at_two_half_lives() {
        let half_life = Some(30.0);
        let now = 86400 * 60; // 60 days
        let created = 0;

        let multiplier = temporal_decay_multiplier("session", created, now, half_life);
        assert!(
            (multiplier - 0.25).abs() < 0.01,
            "60-day-old session chunk should score ~0.25, got {multiplier}"
        );
    }

    #[test]
    fn test_fresh_session_chunk_no_decay() {
        let half_life = Some(30.0);
        let now = 1_000_000;
        let created = now; // just created

        let multiplier = temporal_decay_multiplier("session", created, now, half_life);
        assert!(
            (multiplier - 1.0).abs() < f64::EPSILON,
            "brand-new session chunk should have multiplier ~1.0, got {multiplier}"
        );
    }

    #[test]
    fn test_zero_half_life_returns_one() {
        let multiplier = temporal_decay_multiplier("session", 0, 86400 * 30, Some(0.0));
        assert_eq!(multiplier, 1.0, "zero half-life should disable decay");
    }

    #[test]
    fn test_negative_half_life_returns_one() {
        let multiplier = temporal_decay_multiplier("session", 0, 86400 * 30, Some(-5.0));
        assert_eq!(multiplier, 1.0, "negative half-life should disable decay");
    }

    #[test]
    fn test_future_created_at_no_negative_age() {
        let now = 1_000_000;
        let created = now + 86400; // 1 day in the future (clock skew)
        let half_life = Some(30.0);

        let multiplier = temporal_decay_multiplier("session", created, now, half_life);
        assert!(
            (multiplier - 1.0).abs() < f64::EPSILON,
            "future created_at should clamp age to 0, got {multiplier}"
        );
    }

    #[tokio::test]
    async fn test_hybrid_search_old_session_ranks_below_evergreen() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        // Index a workspace file (evergreen) and a session file (decays)
        let ws_file = tmp.path().join("ws.md");
        std::fs::write(&ws_file, "# WS\n\nRust workspace content about memory.").unwrap();
        idx.reindex_file(&ws_file, "workspace").unwrap();

        let sess_file = tmp.path().join("sess.md");
        std::fs::write(&sess_file, "# Sess\n\nRust session content about memory.").unwrap();
        idx.reindex_file(&sess_file, "session").unwrap();

        // Backdate the session chunk's created_at by 60 days (2 half-lives)
        let sixty_days_ago = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 86400 * 60;
        let sess_path = sess_file.to_string_lossy().to_string();
        idx.db()
            .execute(
                "UPDATE chunks SET created_at = ?1 WHERE path = ?2",
                rusqlite::params![sixty_days_ago, sess_path],
            )
            .unwrap();

        // Equal source weights to isolate temporal decay
        let mut source_weights = std::collections::HashMap::new();
        source_weights.insert("workspace".to_string(), 1.0);
        source_weights.insert("session".to_string(), 1.0);

        let config = MemorySearchConfig {
            min_score: 0.0,
            source_weights,
            ..Default::default()
        };

        let results = hybrid_search(&idx, None, "rust memory", &config)
            .await
            .unwrap();

        assert!(results.len() >= 2, "should find both chunks");
        let ws = results.iter().find(|r| r.source == "workspace").unwrap();
        let sess = results.iter().find(|r| r.source == "session").unwrap();

        // Workspace (evergreen) should rank above the 60-day-old session chunk.
        // At 2 half-lives the session chunk decays to ~0.25× its base score,
        // while the workspace chunk stays at 1.0×.
        assert!(
            ws.score > sess.score,
            "evergreen workspace ({:.4}) should outscore 60-day-old session ({:.4})",
            ws.score,
            sess.score,
        );
    }

    // -----------------------------------------------------------------------
    // PR-8: access-frequency boost tests
    // -----------------------------------------------------------------------

    /// A chunk with access_count > 0 scores higher than an identical chunk
    /// with access_count = 0, all else equal.
    #[tokio::test]
    async fn test_access_boost_raises_frequently_accessed_chunks() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        // Two files with nearly identical content; chunk B is accessed once.
        let fa = tmp.path().join("chunk_a.md");
        let fb = tmp.path().join("chunk_b.md");
        std::fs::write(&fa, "# Rust\n\nRust ownership model explained.").unwrap();
        std::fs::write(&fb, "# Rust\n\nRust ownership model explained.").unwrap();
        idx.reindex_file(&fa, "workspace").unwrap();
        idx.reindex_file(&fb, "workspace").unwrap();

        // Record one access for chunk B.
        let chunk_b_id = format!("{}:0", fb.to_string_lossy());
        idx.record_access(&chunk_b_id).unwrap();

        // Use the DEFAULT config (all source_weights = 1.0). Both chunks
        // normalize to base_score = 1.0 as the top FTS matches, so their
        // display scores both clamp to 1.0 — but ranking is performed on the
        // UNCLAMPED score, so the access boost still orders the accessed chunk
        // first. This exercises the common default-config path where the clamp
        // would otherwise make the boost inert.
        let config = MemorySearchConfig::default();
        let results = hybrid_search_merge(
            &idx,
            idx.search_fts("rust ownership", 10).unwrap(),
            None,
            &config,
        )
        .unwrap();

        // Both chunks must be returned (no vacuous "inconclusive → pass" path).
        let pos_a = results
            .iter()
            .position(|r| r.path == fa.to_string_lossy().as_ref())
            .expect("chunk A must be returned");
        let pos_b = results
            .iter()
            .position(|r| r.path == fb.to_string_lossy().as_ref())
            .expect("chunk B must be returned");

        // The accessed chunk (B) must rank ahead of the unaccessed chunk (A),
        // even though both display scores clamp to 1.0 under default weights.
        assert!(
            pos_b < pos_a,
            "accessed chunk (rank {pos_b}) should rank ahead of unaccessed (rank {pos_a})",
        );
        // Pin the premise that makes rank-on-unclamped necessary: BOTH display
        // scores are exactly 1.0 (the collision the split resolves). The rank
        // ordering above therefore can only come from the unclamped score.
        assert!(
            (results[pos_a].score - 1.0).abs() < 1e-9,
            "unaccessed display score ({:.6}) must clamp to exactly 1.0",
            results[pos_a].score,
        );
        assert!(
            (results[pos_b].score - 1.0).abs() < 1e-9,
            "accessed display score ({:.6}) must clamp to exactly 1.0",
            results[pos_b].score,
        );
    }

    /// Covers the MMR-enabled handoff through `hybrid_search_merge` — the
    /// construction and alignment of the `relevance`/`results` vectors — which
    /// no other search test exercises (MMR is off by default).
    ///
    /// This is NOT the raw-vs-clamped regression guard: `results` enters MMR
    /// pre-sorted by `raw_score`, so the boosted chunk would stay first even on
    /// a buggy `.score` read. That guarantee lives in the unit test
    /// `test_mmr_ranks_on_relevance_not_clamped_score`.
    #[tokio::test]
    async fn test_hybrid_search_merge_with_mmr_enabled() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        // Two identical (redundant) chunks + one diverse chunk, all matching.
        let fa = tmp.path().join("a.md");
        let fb = tmp.path().join("b.md");
        let fc = tmp.path().join("c.md");
        std::fs::write(&fa, "# Rust\n\nRust ownership model explained.").unwrap();
        std::fs::write(&fb, "# Rust\n\nRust ownership model explained.").unwrap();
        std::fs::write(&fc, "# Borrow\n\nRust borrowing and lifetimes guide.").unwrap();
        idx.reindex_file(&fa, "workspace").unwrap();
        idx.reindex_file(&fb, "workspace").unwrap();
        idx.reindex_file(&fc, "workspace").unwrap();

        // Boost chunk B so its unclamped relevance exceeds chunk A's.
        let chunk_b_id = format!("{}:0", fb.to_string_lossy());
        idx.record_access(&chunk_b_id).unwrap();

        // Enable MMR (relevance-leaning lambda) to drive the aligned handoff.
        let mut config = MemorySearchConfig {
            min_score: 0.0,
            ..Default::default()
        };
        config.mmr.enabled = true;
        config.mmr.lambda = 0.7;

        let results = hybrid_search_merge(
            &idx,
            idx.search_fts("rust ownership", 10).unwrap(),
            None,
            &config,
        )
        .unwrap();

        assert!(
            !results.is_empty(),
            "MMR-enabled search must return results"
        );
        let pos_a = results
            .iter()
            .position(|r| r.path == fa.to_string_lossy().as_ref())
            .expect("chunk A must be returned");
        let pos_b = results
            .iter()
            .position(|r| r.path == fb.to_string_lossy().as_ref())
            .expect("chunk B must be returned");

        // Through the MMR handoff, the access-boosted chunk (B) ranks ahead of
        // its identical twin (A) because MMR's relevance term reads the
        // unclamped `relevance` slice (both share a clamped display score of 1.0).
        assert!(
            pos_b < pos_a,
            "boosted chunk (rank {pos_b}) should rank ahead of its twin (rank {pos_a}) with MMR on",
        );
    }

    /// access_boost never penalises zero-access chunks (boost = 1.0 for access_count = 0).
    #[test]
    fn test_access_boost_zero_access_is_neutral() {
        let boost = 1.0 + (0_f64).ln_1p() * 0.05;
        assert!(
            (boost - 1.0).abs() < f64::EPSILON,
            "zero accesses must yield a boost factor of exactly 1.0"
        );
    }

    // -----------------------------------------------------------------------
    // PR: scoring normalization fix tests
    // -----------------------------------------------------------------------

    /// FTS-only results (no vector search) should score well above a
    /// reasonable min_score threshold (e.g., 0.3). Before the fix,
    /// FTS-only chunks in hybrid mode had their scores capped at
    /// text_weight (0.3), making them impossible to retrieve at default
    /// min_score = 0.35.
    #[tokio::test]
    async fn test_fts_only_scores_above_reasonable_threshold() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let file_path = tmp.path().join("test.md");
        std::fs::write(
            &file_path,
            "# Rust Guide\n\nRust programming language ownership and borrowing tutorial.",
        )
        .unwrap();
        idx.reindex_file(&file_path, "workspace").unwrap();

        let config = MemorySearchConfig {
            min_score: 0.3,
            ..Default::default()
        };

        let results = hybrid_search(&idx, None, "rust programming", &config)
            .await
            .unwrap();

        assert!(
            !results.is_empty(),
            "FTS-only results must pass min_score=0.3 threshold"
        );
        assert!(
            results[0].score > 0.3,
            "FTS-only score ({:.4}) must exceed 0.3",
            results[0].score,
        );
    }

    /// Global MEMORY.md chunks (source_weight = 0.7) should still be
    /// retrievable with a reasonable threshold. Before the fix, global
    /// chunks were capped at text_weight × source_weight = 0.21, making
    /// them invisible at any threshold above 0.2.
    #[tokio::test]
    async fn test_global_source_scores_above_min_threshold() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let file_path = tmp.path().join("global.md");
        std::fs::write(
            &file_path,
            "# Project Conventions\n\nAlways use graphite for PRs. Never commit without review.",
        )
        .unwrap();
        idx.reindex_file(&file_path, "global").unwrap();

        let config = MemorySearchConfig {
            min_score: 0.25,
            ..Default::default()
        };

        let results = hybrid_search(&idx, None, "graphite PRs review", &config)
            .await
            .unwrap();

        assert!(
            !results.is_empty(),
            "global source results must pass min_score=0.25 threshold"
        );
        assert!(
            results[0].score > 0.25,
            "global chunk score ({:.4}) must exceed 0.25",
            results[0].score,
        );
    }

    /// When vector results exist for some chunks but not others, FTS-only
    /// chunks should NOT be penalized. Their FTS score should remain at
    /// full weight (1.0 × normalized), not capped at text_weight.
    #[tokio::test]
    async fn test_fts_only_chunks_not_penalized_by_vec_existence() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);
        let mock = MockEmbeddingProvider { dimensions: 4 };

        // File A: has both FTS and vector embedding
        let file_a = tmp.path().join("embedded.md");
        std::fs::write(
            &file_a,
            "# Rust\n\nRust programming language ownership tutorial.",
        )
        .unwrap();
        idx.reindex_file(&file_a, "workspace").unwrap();

        // Embed chunk A
        let path_a = file_a.to_string_lossy().to_string();
        let chunk_a_id = format!("{path_a}:0");
        let chunk_a = idx.get_chunk(&chunk_a_id).unwrap().unwrap();
        let embeddings = mock.embed_batch(&[&chunk_a.text]).await.unwrap();
        idx.upsert_embedding(&chunk_a_id, &embeddings[0]).unwrap();

        // File B: FTS only (no embedding)
        let file_b = tmp.path().join("unembedded.md");
        std::fs::write(
            &file_b,
            "# Rust\n\nRust programming language borrowing tutorial.",
        )
        .unwrap();
        idx.reindex_file(&file_b, "workspace").unwrap();

        let config = MemorySearchConfig {
            min_score: 0.0,
            ..Default::default()
        };

        // Use mock provider so hybrid path runs vector search
        let results = hybrid_search(
            &idx,
            Some(&mock as &dyn EmbeddingProvider),
            "rust programming",
            &config,
        )
        .await
        .unwrap();

        // Both chunks must be found
        let result_b = results
            .iter()
            .find(|r| r.path == file_b.to_string_lossy().as_ref());

        assert!(
            result_b.is_some(),
            "FTS-only chunk must appear in results even when other chunks have vectors"
        );
        let score_b = result_b.unwrap().score;
        assert!(
            score_b > 0.3,
            "FTS-only chunk score ({:.4}) must not be penalized below 0.3 by vec existence",
            score_b,
        );
    }

    /// Vector normalization should use absolute L2 distance scale (max = 2.0)
    /// instead of relative normalization. This ensures that even when all
    /// candidates have similar distances, vector scores still contribute
    /// meaningfully.
    ///
    /// Note: `dimensions: 4` is chosen deliberately. The mock provider
    /// (blake3 bytes / 255.0) does NOT produce unit-norm vectors. At low
    /// dimensions the L2 distances stay within `MAX_L2_DISTANCE = 2.0`, so
    /// the absolute normalization works. At production dimensions (1024),
    /// mock distances could exceed 2.0 and clamp to 0 — use real embeddings
    /// or normalize the mock output for high-dimensional tests.
    #[tokio::test]
    async fn test_vector_absolute_normalization() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);
        let mock = MockEmbeddingProvider { dimensions: 4 };

        let file = tmp.path().join("test.md");
        std::fs::write(&file, "# Test\n\nContent for vector search test.").unwrap();
        idx.reindex_file(&file, "workspace").unwrap();

        let path_str = file.to_string_lossy().to_string();
        let chunk_id = format!("{path_str}:0");

        // Use the mock to get a consistent embedding
        let embedding = mock.embed_batch(&["test"]).await.unwrap();
        idx.upsert_embedding(&chunk_id, &embedding[0]).unwrap();

        // Search with vector — the mock returns deterministic embeddings
        let fts_results = idx.search_fts("content test", 10).unwrap_or_default();
        let query_embedding = mock.embed_batch(&["content test"]).await.unwrap();

        let config = MemorySearchConfig {
            min_score: 0.0,
            ..Default::default()
        };

        let results =
            hybrid_search_merge(&idx, fts_results, Some(&query_embedding[0]), &config).unwrap();

        assert!(!results.is_empty(), "should find at least one result");
        // With absolute normalization, the combined score should be
        // substantially above zero (mock embeddings produce deterministic
        // but varying values).
        assert!(
            results[0].score > 0.1,
            "hybrid score ({:.4}) should be meaningful with absolute normalization",
            results[0].score,
        );
    }

    // -----------------------------------------------------------------------
    // Empty-template filter + score clamp tests
    // -----------------------------------------------------------------------

    /// The auto-generated global MEMORY.md stub, written verbatim by
    /// `MemoryStorage::ensure_initialized` (storage.rs), including the trailing
    /// newline. Kept in sync with that source.
    const GLOBAL_STUB: &str = "# Global Memory\n\
         \n\
         > This file is automatically managed by Axon's memory system.\n\
         > You can also edit it manually — changes will be indexed on next session.\n\
         \n\
         ## Preferences\n\
         \n\
         <!-- Add any cross-project preferences here -->\n";

    /// The shorter workspace stub variant from `dream.rs`.
    const WORKSPACE_STUB: &str =
        "# Project Memory — /test\n\n> Auto-populated by dream consolidation. Edit freely.\n";

    #[test]
    fn test_is_content_free_global_stub() {
        // Caught via the marker-based scaffold predicate (it has blockquote
        // disclaimer lines, so it is NOT structurally empty) — only on
        // evergreen sources, where the stubs live.
        assert!(
            is_content_free(GLOBAL_STUB, "global"),
            "the unedited global MEMORY.md stub must be content-free"
        );
        assert!(
            is_content_free(WORKSPACE_STUB, "workspace"),
            "the auto-generated workspace stub must be content-free"
        );
    }

    #[test]
    fn test_is_content_free_scaffolding_only() {
        // The structural branch applies to ALL sources — use "session" here.
        assert!(
            is_content_free("# Heading\n## Subheading", "session"),
            "headings only"
        );
        assert!(
            is_content_free("<!-- just a comment -->", "session"),
            "comment only (single line)"
        );
        assert!(
            is_content_free("<!--\nmulti\nline\ncomment\n-->", "session"),
            "comment only (multi-line)"
        );
        assert!(is_content_free("", "session"), "empty string");
        assert!(is_content_free("   \n\t\n  ", "session"), "whitespace only");
        assert!(
            is_content_free("# Heading\n\n<!-- a comment -->\n\n## Another", "session"),
            "headings + comments only"
        );
        assert!(
            is_content_free("   # Indented Heading", "session"),
            "indented ATX heading is still a heading"
        );
    }

    #[test]
    fn test_is_content_free_real_content() {
        // Use "global" (evergreen) so both filter branches are active; real
        // content must survive regardless.
        assert!(
            !is_content_free("## Preferences\n\n- Use tabs", "global"),
            "heading with a following bullet has real content"
        );
        assert!(
            !is_content_free("Use C# for this", "global"),
            "a `#` mid-line is real content, not a heading"
        );
        assert!(
            !is_content_free("#hashtag not a heading", "global"),
            "`#` with no following space is not an ATX heading (per header_level)"
        );
        assert!(
            !is_content_free("# Title\n\nSome actual prose here.", "global"),
            "prose after a heading is real content"
        );
        assert!(
            !is_content_free("<!-- comment -->\nactual content", "global"),
            "content after a comment counts"
        );
        assert!(
            !is_content_free("- a\n- b", "global"),
            "list-only chunk is content"
        );
        assert!(
            !is_content_free("Title\n=====", "global"),
            "setext heading underline counts as content"
        );
        assert!(
            !is_content_free("```\nlet x = 1;\n```", "global"),
            "code-fence chunk is content"
        );
    }

    /// The scaffold-marker branch is scoped to evergreen sources: a short
    /// non-evergreen chunk that merely quotes a marker phrase must be kept,
    /// while the same text on an evergreen source is filtered.
    #[test]
    fn test_is_content_free_marker_branch_scoped_to_evergreen() {
        // A short session note that happens to quote a scaffold marker phrase.
        let quotes_marker =
            "Reminder: the template says \"Add any cross-project preferences here\".";
        assert!(
            !is_content_free(quotes_marker, "session"),
            "non-evergreen chunk quoting a marker phrase must NOT be filtered"
        );
        assert!(
            is_content_free(quotes_marker, "global"),
            "the same short text on an evergreen source is treated as scaffold"
        );
    }

    /// Blockquotes are real user content and must NOT be filtered (the user's
    /// spec listed only headings/comments/whitespace as scaffolding).
    #[test]
    fn test_is_content_free_preserves_blockquotes() {
        assert!(
            !is_content_free("> a quote\n> another quote", "global"),
            "blockquote-only user notes must be preserved"
        );
        assert!(
            !is_content_free(
                "## Important\n> Always run migrations before deploy",
                "global"
            ),
            "heading + blockquote with real guidance must be preserved"
        );
    }

    /// An unterminated `<!--` keeps the remainder as literal text, so a chunk
    /// with real content around it is not classified content-free.
    #[test]
    fn test_is_content_free_unterminated_comment_keeps_content() {
        assert!(
            !is_content_free("real text\n<!-- unterminated", "global"),
            "content before an unterminated comment must be kept"
        );
        assert!(
            !is_content_free("<!-- unterminated comment, no closer\nmore text", "global"),
            "content after an unterminated comment must be kept"
        );
    }

    /// An essentially-empty boilerplate chunk must be excluded from results
    /// even though it matches FTS, while a real-content chunk in the same
    /// scenario IS returned (proving the FTS pipeline is non-empty and the
    /// filter — not an empty result set — is what removes the stub).
    #[tokio::test]
    async fn test_content_free_chunk_excluded_from_search() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let stub_path = tmp.path().join("stub.md");
        std::fs::write(&stub_path, GLOBAL_STUB).unwrap();
        idx.reindex_file(&stub_path, "global").unwrap();

        // A real-content global file that also matches the query.
        let real_path = tmp.path().join("real.md");
        std::fs::write(
            &real_path,
            "# Conventions\n\nProject preferences: always use graphite for PRs. \
             Architecture is event-driven.",
        )
        .unwrap();
        idx.reindex_file(&real_path, "global").unwrap();

        // Precondition: the stub IS a raw FTS candidate for this query (the
        // term "preferences" appears in it). This proves the filter — not a
        // non-match — is what removes it from the final results below.
        let fts_candidates = idx
            .search_fts("project conventions preferences architecture", 10)
            .unwrap();
        assert!(
            fts_candidates
                .iter()
                .any(|r| r.chunk_id.starts_with(stub_path.to_string_lossy().as_ref())),
            "stub must be a raw FTS candidate before filtering"
        );

        let config = MemorySearchConfig {
            min_score: 0.0, // accept all by score, so only the filter can exclude
            ..Default::default()
        };

        let results = hybrid_search(
            &idx,
            None,
            "project conventions preferences architecture",
            &config,
        )
        .await
        .unwrap();

        assert!(
            !results.is_empty(),
            "real-content chunk must keep the result set non-empty"
        );
        assert!(
            results
                .iter()
                .any(|r| r.path == real_path.to_string_lossy().as_ref()),
            "real-content global file must be returned"
        );
        assert!(
            results
                .iter()
                .all(|r| r.path != stub_path.to_string_lossy().as_ref()),
            "content-free global stub must be excluded from results",
        );
    }

    /// The display score must clamp to exactly 1.0 when the access boost pushes
    /// the unclamped product above 1.0 — while the unclamped product (used for
    /// ranking) is genuinely > 1.0 (precondition, asserted explicitly so the
    /// test can't silently go vacuous).
    #[tokio::test]
    async fn test_final_score_clamped_to_one() {
        // Precondition: the boost at 100 accesses really does exceed 1.0.
        let boost_at_100 = 1.0 + (100_f64).ln_1p() * 0.05;
        assert!(
            boost_at_100 > 1.0,
            "test precondition: access boost at 100 accesses ({boost_at_100:.4}) must exceed 1.0"
        );

        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let file_path = tmp.path().join("test.md");
        std::fs::write(
            &file_path,
            "# Rust\n\nRust ownership and borrowing tutorial.",
        )
        .unwrap();
        idx.reindex_file(&file_path, "workspace").unwrap();

        // Drive access_count high so the unbounded access_boost exceeds 1.0.
        let chunk_id = format!("{}:0", file_path.to_string_lossy());
        for _ in 0..100 {
            idx.record_access(&chunk_id).unwrap();
        }

        let config = MemorySearchConfig {
            min_score: 0.0,
            ..Default::default()
        };

        let results = hybrid_search_merge(
            &idx,
            idx.search_fts("rust ownership", 10).unwrap(),
            None,
            &config,
        )
        .unwrap();

        assert!(
            !results.is_empty(),
            "should find the frequently-accessed chunk"
        );
        // The top chunk is a top FTS match (base 1.0) × workspace weight (1.0)
        // × boost (>1.0) → unclamped > 1.0 → display score clamped to exactly 1.0.
        assert!(
            (results[0].score - 1.0).abs() < 1e-9,
            "display score ({:.6}) must clamp to exactly 1.0",
            results[0].score,
        );
        for r in &results {
            assert!(
                r.score <= 1.0,
                "score ({:.4}) must be clamped to <= 1.0 despite access boost",
                r.score,
            );
        }
    }

    // ── Reciprocal Rank Fusion ────────────────────────────────────────────

    use crate::embedding::ApiEmbeddingProvider;
    use axon_config_types::SearchFusion;

    fn fts(ids: &[(&str, f64)]) -> Vec<crate::index::FtsResult> {
        ids.iter()
            .map(|(id, rank)| crate::index::FtsResult {
                chunk_id: (*id).to_string(),
                rowid: 0,
                rank: *rank,
            })
            .collect()
    }

    /// `fuse_rrf` against a throwaway empty index. The synthetic chunk ids here
    /// resolve to no row, so every source weight is the neutral 1.0 and these
    /// cases measure pure rank behaviour — which is what they are for. Weighting
    /// is covered by `rrf_*_global_*`, which uses a real corpus.
    fn rrf_of(
        fts_list: &[(&str, f64)],
        vecs: &[(String, f32)],
        cfg: &MemorySearchConfig,
    ) -> std::collections::HashMap<String, f64> {
        let tmp = TempDir::new().unwrap();
        let idx = test_index(&tmp);
        fuse_rrf(&idx, &fts(fts_list), vecs, cfg)
    }

    fn rrf_config(text_weight: f32, vector_weight: f32) -> MemorySearchConfig {
        MemorySearchConfig {
            fusion: SearchFusion::Rrf,
            text_weight,
            vector_weight,
            ..Default::default()
        }
    }

    #[test]
    fn rrf_preserves_list_order_and_normalizes_best_to_one() {
        // Single list, so fusion is just its order.
        let fused = rrf_of(
            &[("a", -5.0), ("b", -3.0), ("c", -1.0)],
            &[],
            &rrf_config(1.0, 0.0),
        );

        assert!(
            (fused["a"] - 1.0).abs() < 1e-12,
            "best must normalize to 1.0"
        );
        assert!(fused["a"] > fused["b"] && fused["b"] > fused["c"]);
    }

    #[test]
    fn rrf_ignores_score_scale_and_reads_only_rank() {
        // Same ORDER, wildly different BM25 magnitudes. RRF must not care:
        // this is the property the weighted path cannot have, because it
        // min-max normalizes within the result set.
        let tight = rrf_of(&[("a", -5.0), ("b", -4.9)], &[], &rrf_config(1.0, 0.0));
        let wide = rrf_of(&[("a", -900.0), ("b", -0.1)], &[], &rrf_config(1.0, 0.0));

        assert!((tight["a"] - wide["a"]).abs() < 1e-12);
        assert!((tight["b"] - wide["b"]).abs() < 1e-12);
    }

    #[test]
    fn rrf_rewards_agreement_between_the_two_lists() {
        // `b` is mid-ranked in both lists; `a` is top of one and absent from
        // the other. Appearing in both is what RRF is for.
        let fused = rrf_of(
            &[("a", -9.0), ("b", -8.0)],
            &[("b".to_string(), 0.1), ("c".to_string(), 0.2)],
            &rrf_config(0.5, 0.5),
        );
        assert!(
            fused["b"] > fused["a"] && fused["b"] > fused["c"],
            "a chunk in both lists must outrank chunks in only one: {fused:?}"
        );
    }

    #[test]
    fn rrf_worst_fts_hit_keeps_a_nonzero_score() {
        // The weighted path min-max normalizes, so the LAST FTS result is
        // always exactly 0.0 however good it was. Under RRF it is merely
        // last, which is what lets source weighting scale it without
        // annihilating it.
        let fused = rrf_of(
            &[("a", -5.0), ("b", -4.0), ("c", -3.0)],
            &[],
            &rrf_config(1.0, 0.0),
        );
        assert!(fused["c"] > 0.0, "last hit must not be zeroed: {fused:?}");
    }

    #[test]
    fn rrf_k_falls_back_to_the_paper_default_when_unusable() {
        let good = rrf_of(&[("a", -5.0), ("b", -4.0)], &[], &rrf_config(1.0, 0.0));
        for bad_k in [-1.0_f32, f32::NAN, f32::INFINITY] {
            let cfg = MemorySearchConfig {
                rrf_k: bad_k,
                ..rrf_config(1.0, 0.0)
            };
            let got = rrf_of(&[("a", -5.0), ("b", -4.0)], &[], &cfg);
            assert!(
                (got["b"] - good["b"]).abs() < 1e-12,
                "rrf_k={bad_k} must fall back to 60, got {got:?}"
            );
        }
    }

    #[test]
    fn rrf_on_empty_input_is_empty_not_a_division_by_zero() {
        assert!(rrf_of(&[], &[], &rrf_config(1.0, 0.0)).is_empty());
    }

    /// The pathology RRF exists to prevent, stated as a test.
    ///
    /// A `global` chunk carries `source_weight = 0.7`. Under the weighted path
    /// its base score is min-max normalized within the FTS result set, so a
    /// chunk that is merely *not the single best match* can be scaled toward
    /// zero and then multiplied by 0.7 — which is how global chunks once got
    /// capped at 0.21 and became invisible above `min_score = 0.2`.
    ///
    /// Under RRF the base score is rank-derived and the set is renormalized so
    /// the best is 1.0, so a highly-ranked global chunk survives its source
    /// weight. Asserted through the real search path, not just `fuse_rrf`.
    ///
    /// ⚠ **This passing does NOT mean RRF retrieves better.** It is a
    /// two-file corpus testing presence above a threshold, not rank
    /// competition among many candidates. `fusion_ab_report` on a 23-file
    /// corpus shows the opposite: at the default k=60, RRF compresses scores
    /// into a ~0.07 band, so the post-hoc `source_weight` multiplier
    /// dominates ranking and global chunks are crowded out entirely (0/12
    /// queries vs 9/12 for weighted). Read the two together.
    #[tokio::test]
    async fn rrf_keeps_a_weighted_down_global_source_above_min_score() {
        let tmp = TempDir::new().unwrap();
        let mut idx = test_index(&tmp);

        let global = tmp.path().join("global.md");
        std::fs::write(
            &global,
            "# Project Conventions\n\nAlways use graphite for PRs. Never commit without review.",
        )
        .unwrap();
        idx.reindex_file(&global, "global").unwrap();

        // A second, better-matching chunk so the global one is NOT the top FTS
        // hit — the exact situation in which min-max normalization pushes it
        // down toward zero.
        let other = tmp.path().join("session.md");
        std::fs::write(
            &other,
            "# Graphite\n\ngraphite graphite graphite PRs PRs review review stacked diffs.",
        )
        .unwrap();
        idx.reindex_file(&other, "session").unwrap();

        let mut source_weights = std::collections::HashMap::new();
        source_weights.insert("global".to_string(), 0.7);
        source_weights.insert("session".to_string(), 1.0);

        let config = MemorySearchConfig {
            fusion: SearchFusion::Rrf,
            min_score: 0.35,
            source_weights,
            ..Default::default()
        };

        let results = hybrid_search(&idx, None, "graphite PRs review", &config)
            .await
            .unwrap();

        assert!(
            results.iter().any(|r| r.source == "global"),
            "a down-weighted global chunk must still clear min_score under RRF; got {:?}",
            results
                .iter()
                .map(|r| (r.source.as_str(), r.score))
                .collect::<Vec<_>>()
        );
    }

    /// The default must stay `Weighted`: this is a prototype behind a flag,
    /// and flipping the default would silently re-rank every existing install.
    #[test]
    fn fusion_defaults_to_weighted() {
        assert_eq!(MemorySearchConfig::default().fusion, SearchFusion::Weighted);
        assert!((MemorySearchConfig::default().rrf_k - 60.0).abs() < f32::EPSILON);
    }

    // ── A/B harness: weighted fusion vs RRF ───────────────────────────────
    //
    // `#[ignore]`d — needs a corpus on disk and prints a report rather than
    // asserting a threshold:
    //
    //   AB_CORPUS=/path/to/markdown/dir \
    //     cargo test -p axon-memory fusion_ab_report -- --ignored --nocapture
    //
    // ⚠ **What this measures and what it does not.** With no embedding
    // provider both arms run FTS-only, and then nothing here is evidence about
    // two-list fusion quality — which is RRF's whole claim. Set `AB_EMBED_URL`
    // (see `ab_embed_provider`) and check the printed `embedded` count: an
    // endpoint can be reachable while the vector table stays empty, which
    // looks exactly like bad retrieval.
    //
    // **Two query sets run, and the difference between them is the point.**
    //
    // - `keyword` — the original 12. Written against file *topics*, so each
    //   query shares distinctive vocabulary with its answer. That is precisely
    //   what BM25 is best at, so the vector arm has nothing to add and these
    //   numbers cannot tell you whether a second list helps.
    // - `paraphrase` — 25 queries built to be semantically clear but
    //   **lexically disjoint** from their answers: no query term that occurs in
    //   five or fewer corpus files also occurs in any accepted answer file.
    //   Checked mechanically over the corpus, not by eye. This is the set where
    //   embeddings can actually contribute, so it is the one that decides.
    //
    // Ground truth is **set-valued**: where topics genuinely overlap, every
    // acceptable file is listed rather than pretending one is uniquely right,
    // and a hit is the best rank among them. Each label was derived by reading
    // the target and confirming it answers the query — not recalled.
    //
    // `MEMORY.md` is deliberately never an accepted answer. It is an index of
    // one-line hooks over the other files, so it nominally "contains" most
    // answers; counting it correct would make the metric meaningless. It stays
    // in the corpus as a distractor, and as the `global` source whose
    // visibility is tracked separately below.
    //
    // **Replicated on a second corpus (2026-09-01).** Different domain and
    // density: the pager user guide (27 files, 10–40 KB each) as `global`
    // and the oh-my-axon agent/skill files (9) as `workspace` — 36 files,
    // 960 chunks, real 768-d vectors. 14 keyword + 24 paraphrase queries,
    // supplied via `AB_QUERIES` and disjointness-checked the same way. At
    // `min_score=0.35`:
    //
    //   set         weighted        rrf k=60        rrf k=1
    //   keyword     100% / 0.964    100% / 0.905    100% / 0.905
    //   paraphrase   50% / 0.299     83% / 0.608     83% / 0.602
    //
    // Paired vs weighted: paraphrase rrf k=60 **15-1** (sign p=0.0005),
    // rrf k=1 **15-2** (p=0.002). Keyword: 0-2, both losses a slip from
    // rank 1→3 or 2→3, so recall is untouched and the MRR gap is that. Every
    // arm keeps `global` visible on 24/24 paraphrase queries, so the source
    // vanishing bug never triggers here — the corpus-1 keyword loss for k=60
    // (92%) did not recur either. Same shape as corpus 1: RRF gives up a
    // little top-rank precision on vocabulary-matched queries and roughly
    // doubles MRR where the two lists disagree. k=1 vs k=60 is a wash on
    // this corpus (k=1 wins recall ungated 88% vs 83%; k=60 wins one MRR
    // point gated) and k=1 was strictly better on corpus 1, so k=1 stays the
    // recommendation.

    /// A labelled query: the text, and the files any of which is an
    /// acceptable answer. More than one is not hedging — it is the honest
    /// label when several documents genuinely answer the same question.
    struct AbCase {
        query: &'static str,
        expect_files: &'static [&'static str],
    }

    /// A named query set. Reported separately; see the note above for why the
    /// two sets are not interchangeable.
    struct AbSet {
        name: &'static str,
        cases: &'static [AbCase],
    }

    const AB_CASES: &[AbCase] = &[
        AbCase {
            query: "where is the live Axon install",
            expect_files: &["axon-live-install.md"],
        },
        AbCase {
            query: "can cargo test run on Windows NASM",
            expect_files: &["axon-testing.md"],
        },
        AbCase {
            query: "what does capabilityMode enforce in an agent file",
            expect_files: &["oh-my-axon.md"],
        },
        AbCase {
            query: "how should I wait for CI to finish",
            expect_files: &["no-jq-on-this-box.md"],
        },
        AbCase {
            query: "what CRF for h264 video output",
            expect_files: &["video-output-crf18.md"],
        },
        AbCase {
            query: "which upscaler should I prefer",
            expect_files: &["upscaling-prefer-ltx25.md"],
        },
        AbCase {
            query: "comfyui boot flags pinned memory",
            expect_files: &["comfyui-axon-integration.md"],
        },
        AbCase {
            query: "how do I pin framing in an H3 shot",
            expect_files: &["h3-shot-authoring.md"],
        },
        AbCase {
            query: "what is the merge convention for main",
            expect_files: &["axon-project.md"],
        },
        AbCase {
            query: "does prompt form change rule following",
            expect_files: &["prompt-form-experiment.md"],
        },
        AbCase {
            query: "first run wizard port scanning localhost",
            expect_files: &["axon-wizard.md"],
        },
        AbCase {
            query: "does graphify support rust",
            expect_files: &["graphify-tool.md"],
        },
    ];

    /// Lexically-disjoint paraphrase queries — the set that can actually
    /// discriminate a two-list fusion. Built 2026-08-31 against the 31-file
    /// corpus (`.axon/{plans,audits,handoffs}/` + the memory dir), because the
    /// labels from the 23-file memory corpus do NOT transfer: terms that pick
    /// out exactly one memory file appear in up to nine files here.
    ///
    /// Disjointness is a checked property, not an intention: no term in a query
    /// occurs in five or fewer corpus files AND in one of its accepted answers.
    const AB_PARAPHRASE_CASES: &[AbCase] = &[
        AbCase {
            query: "why can't I run the unit tests on this machine by themselves",
            expect_files: &["axon-testing.md"],
        },
        AbCase {
            query: "the thing I set up to notice when the remote job went green stayed silent for an hour",
            expect_files: &["no-jq-on-this-box.md"],
        },
        AbCase {
            query: "what number should the encoder be told to use when saving a movie",
            expect_files: &["video-output-crf18.md"],
        },
        AbCase {
            query: "how should I approach a very large document without spending everything on it",
            expect_files: &["context-discipline.md"],
        },
        AbCase {
            query: "the verification came back clean but it was pointed at a place where the defect was impossible",
            expect_files: &["probe-that-cannot-fail.md"],
        },
        AbCase {
            query: "I keep reaching for numbers that miss what the person was actually unhappy about",
            expect_files: &["metric-must-match-the-complaint.md"],
        },
        AbCase {
            query: "how do I know whether something should stay here or go out where strangers can read it",
            expect_files: &["scope-confirm-before-publishing.md"],
        },
        AbCase {
            query: "there are two of these installed and I need to know which one really runs",
            expect_files: &["axon-live-install.md"],
        },
        AbCase {
            query: "which box on the fleet can turn words into numeric representations and which one cannot",
            expect_files: &["model-hosts-and-serving.md", "axon-and-oh-my-axon.md"],
        },
        AbCase {
            query: "when several projects ship the same low level optimisation code whose should be the default choice",
            expect_files: &["prefer-comfy-kitchen.md"],
        },
        AbCase {
            query: "the user is tired of one enlargement method and wants the other one tried instead",
            expect_files: &["upscaling-prefer-ltx25.md"],
        },
        AbCase {
            query: "how do I state what a person's build looks like so the generator actually honours it",
            expect_files: &["h3-shot-authoring.md"],
        },
        AbCase {
            query: "the field that was supposed to restrict what a helper may touch turns out to restrict nothing",
            expect_files: &["oh-my-axon.md", "axon-internals.md"],
        },
        AbCase {
            query: "old notes use words that no longer appear anywhere in the tree where is the mapping",
            expect_files: &["axon-rename-and-purge.md"],
        },
        AbCase {
            query: "should I generate the relationship diagram for everything at once or just one area",
            expect_files: &["graphify-tool.md"],
        },
        AbCase {
            query: "what has to pass before a change can land",
            expect_files: &["axon-repo-hygiene.md", "axon-testing.md"],
        },
        AbCase {
            query: "does the way an instruction is worded change whether a small model obeys it",
            expect_files: &["prompt-form-experiment.md"],
        },
        AbCase {
            query: "on the very first start how does it notice a server already running nearby and what network trap did that hit",
            expect_files: &["axon-wizard.md"],
        },
        AbCase {
            query: "several nearly identical routines pull declarations out of source files do they actually differ",
            expect_files: &["2026-08-29-extractor-recon.md"],
        },
        AbCase {
            query: "the singer's face did not line up with the words being sung and that halted the round",
            expect_files: &[
                "mv-city-of-scarred.md",
                "comfyui-axon-integration.md",
                "comfyui-run-log.md",
            ],
        },
        AbCase {
            query: "how many claims in the shipped written material contradicted the code",
            expect_files: &["2026-08-29-docs-drift-axon.md"],
        },
        AbCase {
            query: "the tally of stored runs did not grow even though the box did plenty of work that day",
            expect_files: &["2026-08-30-sessionend-usage.md"],
        },
        AbCase {
            query: "what has to be written down before everything in this window disappears",
            expect_files: &["handoff-on-restart.md"],
        },
        AbCase {
            query: "the staged plan for finding out how well the assistant does at different jobs",
            expect_files: &["2026-08-29-agent-testing-programme.md"],
        },
        AbCase {
            query: "which launch switches keep the render stack from consuming all the ram",
            expect_files: &[
                "comfyui-axon-integration.md",
                "prefer-comfy-kitchen.md",
                "comfyui-run-log.md",
            ],
        },
    ];

    const AB_SETS: &[AbSet] = &[
        AbSet {
            name: "keyword",
            cases: AB_CASES,
        },
        AbSet {
            name: "paraphrase",
            cases: AB_PARAPHRASE_CASES,
        },
    ];

    /// Owned mirror of [`AbSet`] so a query file can be loaded at runtime.
    struct AbOwnedCase {
        query: String,
        expect_files: Vec<String>,
    }
    struct AbOwnedSet {
        name: String,
        cases: Vec<AbOwnedCase>,
    }

    /// The query sets to run: `AB_QUERIES=<file.json>` if set, else the
    /// built-in sets above.
    ///
    /// The built-in labels are tied to ONE corpus (the 31-file memory +
    /// `.axon/` split), and labels do not transfer — a term that picks out a
    /// single file there can be in nine files elsewhere. A second corpus
    /// therefore needs its own file, shaped as
    ///
    /// ```json
    /// [{"name": "keyword", "cases": [{"query": "...", "expect_files": ["a.md"]}]},
    ///  {"name": "paraphrase", "cases": [...]}]
    /// ```
    ///
    /// The set names are free text but `paraphrase` is the one that carries
    /// the lexical-disjointness claim, and that claim must be checked
    /// mechanically against the corpus before the file is trusted; this
    /// harness only runs what it is given.
    fn ab_sets() -> Vec<AbOwnedSet> {
        if let Some(path) = std::env::var_os("AB_QUERIES") {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("AB_QUERIES {}: {e}", path.to_string_lossy()));
            let v: serde_json::Value = serde_json::from_str(&text).expect("AB_QUERIES is not JSON");
            let sets = v
                .as_array()
                .expect("AB_QUERIES: top level must be an array of sets");
            return sets
                .iter()
                .map(|s| AbOwnedSet {
                    name: s["name"].as_str().expect("set.name").to_string(),
                    cases: s["cases"]
                        .as_array()
                        .expect("set.cases")
                        .iter()
                        .map(|c| AbOwnedCase {
                            query: c["query"].as_str().expect("case.query").to_string(),
                            expect_files: c["expect_files"]
                                .as_array()
                                .expect("case.expect_files")
                                .iter()
                                .map(|f| f.as_str().expect("expect_files entry").to_string())
                                .collect(),
                        })
                        .collect(),
                })
                .collect();
        }
        AB_SETS
            .iter()
            .map(|s| AbOwnedSet {
                name: s.name.to_string(),
                cases: s
                    .cases
                    .iter()
                    .map(|c| AbOwnedCase {
                        query: c.query.to_string(),
                        expect_files: c.expect_files.iter().map(|f| f.to_string()).collect(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Build the real API embedding provider from env, or `None` for FTS-only.
    ///
    /// `AB_EMBED_URL` is an OpenAI-shaped base (the provider appends
    /// `/embeddings`), e.g. a local LM Studio at
    /// `http://127.0.0.1:49152/v1`. `AB_EMBED_MODEL` and `AB_EMBED_DIMS` must
    /// match what that endpoint actually serves — a dimension mismatch is
    /// silent until vectors are compared, so the harness verifies it below
    /// rather than trusting the setting.
    fn ab_embed_provider() -> Option<(ApiEmbeddingProvider, usize)> {
        let url = std::env::var("AB_EMBED_URL").ok()?;
        let model = std::env::var("AB_EMBED_MODEL").ok()?;
        let dims: usize = std::env::var("AB_EMBED_DIMS")
            .ok()
            .and_then(|d| d.parse().ok())
            .unwrap_or(768);
        let cfg = axon_config_types::MemoryEmbeddingConfig {
            provider: "api".to_string(),
            model: Some(model),
            dimensions: dims,
        };
        let key = std::env::var("AB_EMBED_KEY").unwrap_or_else(|_| "lm-studio".to_string());
        ApiEmbeddingProvider::from_session(&cfg, url, key).map(|p| (p, dims))
    }

    /// Embed every chunk that has no vector yet. Returns how many landed.
    async fn ab_embed_all(idx: &mut MemoryIndex, provider: &ApiEmbeddingProvider) -> usize {
        let pending = idx.chunks_without_embeddings().unwrap_or_default();
        let mut done = 0;
        // The provider batches internally at 32; chunk here too so one failed
        // batch does not abandon the rest of the corpus.
        for batch in pending.chunks(32) {
            let texts: Vec<&str> = batch.iter().map(|(_, text)| text.as_str()).collect();
            match provider.embed_batch(&texts).await {
                Ok(vectors) => {
                    for ((chunk_id, _), vector) in batch.iter().zip(vectors) {
                        if idx.upsert_embedding(chunk_id, &vector).is_ok() {
                            done += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  embed batch failed ({e}); continuing FTS-only for those chunks");
                }
            }
        }
        done
    }

    /// ⚠ `min_score` is swept, and it is NOT a cosmetic knob — it is a
    /// confound that would otherwise be read as a fusion result.
    ///
    /// The two arms are not gated alike. The weighted path compares an
    /// ABSOLUTE score against `min_score`, so a true positive can be pruned
    /// outright. `fuse_rrf` renormalizes its output so the best hit is exactly
    /// 1.0, which makes the same `min_score` *relative to the top hit* — and
    /// since RRF scores land in a narrow band, nearly everything survives.
    /// A win measured only at 0.35 could therefore be "RRF's gate is looser",
    /// not "rank-space fusion ranks better". Running at 0.0 removes the gate
    /// from both arms and isolates the fusion.
    fn ab_config(
        fusion: SearchFusion,
        rrf_k: f32,
        min_score: f32,
        vector_weight: f32,
        bm25_saturation: f32,
    ) -> MemorySearchConfig {
        // A realistic weighting in which `global` carries less than 1.0 —
        // the configuration under which the reported pathology appears.
        // Everything except `fusion` is identical across the two arms.
        let mut source_weights = std::collections::HashMap::new();
        source_weights.insert("global".to_string(), 0.7);
        source_weights.insert("workspace".to_string(), 1.0);
        source_weights.insert("session".to_string(), 1.0);

        MemorySearchConfig {
            fusion,
            rrf_k,
            min_score,
            vector_weight,
            bm25_saturation,
            max_results: 6,
            source_weights,
            ..Default::default()
        }
    }

    /// Best (lowest) 1-based rank at which any acceptable answer appears.
    fn ab_rank_of(results: &[SearchResult], expect_files: &[&str]) -> Option<usize> {
        results
            .iter()
            .position(|r| {
                std::path::Path::new(&r.path)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| expect_files.contains(&f))
            })
            .map(|i| i + 1)
    }

    #[tokio::test]
    #[ignore] // needs AB_CORPUS; prints a report rather than asserting
    async fn fusion_ab_report() {
        let corpus = std::env::var_os("AB_CORPUS")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .expect("set AB_CORPUS to a directory of .md files");

        let tmp = TempDir::new().unwrap();
        // Dimensions must match what the endpoint actually serves, so build
        // the provider first and size the index from it. A mismatch is silent
        // until vectors are compared, where it looks like bad retrieval rather
        // than a config error.
        let embed = ab_embed_provider();
        let dims = embed.as_ref().map(|(_, d)| *d).unwrap_or(4);
        let mut idx = {
            init_sqlite_vec();
            let global = tmp.path().join("memory");
            let workspace = global.join("ab_ws");
            let storage = MemoryStorage::with_paths(global, workspace);
            let db = tmp.path().join("ab.sqlite");
            MemoryIndex::open_or_create(&db, storage, MemoryIndexConfig::default(), dims).unwrap()
        };

        // Source assignment. If the corpus has `global/` and `workspace/`
        // subdirectories, they define the sources; otherwise everything is
        // `workspace` except `MEMORY.md`.
        //
        // ⚠ The flat fallback is kept only for older corpora — it makes the
        // `global visible` column MEANINGLESS. It puts exactly one file in
        // `global`, `MEMORY.md`, which ground truth defines as never a correct
        // answer. "Global is visible" and "the right answer ranked well" are
        // then in direct opposition, so the column cannot adjudicate any
        // change: driving the index down looks like a regression and is
        // actually an improvement. The split layout puts the real memory files
        // in `global`, where the down-weighted source holds true answers and
        // the column measures what it was introduced to measure — whether a
        // whole source gets scored away.
        let split = corpus.join("global").is_dir() && corpus.join("workspace").is_dir();
        let mut files: Vec<(std::path::PathBuf, &'static str)> = Vec::new();
        let collect = |dir: &std::path::Path, source: &'static str, out: &mut Vec<_>| {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.filter_map(|e| e.ok()) {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("md") {
                        out.push((p, source));
                    }
                }
            }
        };
        if split {
            collect(&corpus.join("global"), "global", &mut files);
            collect(&corpus.join("workspace"), "workspace", &mut files);
        } else {
            collect(&corpus, "workspace", &mut files);
            for (p, source) in files.iter_mut() {
                if p.file_name().and_then(|f| f.to_str()) == Some("MEMORY.md") {
                    *source = "global";
                }
            }
        }
        files.sort();

        let mut indexed = 0;
        for (path, source) in &files {
            if idx.reindex_file(path, source).is_ok() {
                indexed += 1;
            }
        }

        let embedded = match embed.as_ref() {
            Some((provider, _)) => ab_embed_all(&mut idx, provider).await,
            None => 0,
        };
        let provider_ref: Option<&dyn EmbeddingProvider> =
            embed.as_ref().map(|(p, _)| p as &dyn EmbeddingProvider);

        // ⚠ Fingerprint the corpus, do not just count it. `AB_CORPUS` points
        // at `.axon/handoffs/` + the memory dir by default — the same files
        // this experiment's write-up edits. A run whose corpus drifted under
        // it silently produces numbers that cannot be compared with the last
        // run's, which happened here (445 -> 446 chunks, weighted paraphrase
        // recall 60% -> 64%). Numbers are only ever comparable WITHIN one
        // fingerprint. Freeze the corpus for a real comparison.
        let fingerprint = {
            let mut h: u64 = 1469598103934665603;
            let mut total = 0u64;
            for (f, source) in &files {
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let len = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                h ^= source.len() as u64;
                total += len;
                for b in name.as_bytes().iter().chain(len.to_le_bytes().iter()) {
                    h ^= *b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
            }
            (h, total)
        };

        println!("\n=== fusion A/B ===");
        println!(
            "corpus:  {} ({indexed} files, {} sources)",
            corpus.display(),
            if split {
                "split global/workspace"
            } else {
                "flat — global visible is MEANINGLESS, see note"
            }
        );
        println!(
            "corpus fingerprint: {:016x} ({} bytes) — compare arms only within one fingerprint",
            fingerprint.0, fingerprint.1
        );
        let sets = ab_sets();
        println!(
            "query sets: {} ({})",
            sets.len(),
            std::env::var("AB_QUERIES")
                .map(|p| format!("from {p}"))
                .unwrap_or_else(|_| "built-in".into())
        );
        match embed.as_ref() {
            // `vec_available` is the index's own answer, not the config's:
            // an endpoint can be reachable and the vector table still empty.
            Some((_, d)) => println!(
                "vectors: {embedded} chunks embedded at {d} dims, vec_available={}",
                idx.vec_available()
            ),
            None => println!("vectors: NONE — both arms FTS-only (set AB_EMBED_URL)"),
        }

        for set in &sets {
            let n = set.cases.len() as f64;
            for gate in [0.35f32, 0.0] {
                println!(
                    "\n─── query set: {} ({} queries), min_score={gate}",
                    set.name,
                    set.cases.len()
                );

                let mut summary: Vec<(&str, f64, f64, usize, usize)> = Vec::new();
                // `weighted vw=1.0` is a diagnostic arm, not a candidate. The
                // weighted path scores an FTS-only chunk at its full min-max
                // normalized BM25 — and that normalization is *within the
                // result set*, so the best keyword hit is always exactly 1.0
                // however irrelevant it is — while a vector-only chunk scores
                // `vector_weight × cosine` ≈ 0.7 × 0.65 ≈ 0.45. If that ceiling
                // is what buries the semantically correct document, raising
                // vector_weight to 1.0 should NOT rescue it: 1.0 × cosine still
                // loses to 1.0. If recall jumps instead, the diagnosis is wrong
                // and the weight, not the normalization, was the problem.
                // The `sat=` arms sweep the absolute-BM25 knee rather than
                // picking one. A value that wins only at a hand-chosen k is
                // overfitting; what would justify the change is a broad
                // plateau that beats the baseline on BOTH query sets.
                for (label, fusion, k, vw, sat) in [
                    ("weighted", SearchFusion::Weighted, 60.0, 0.7, 0.0),
                    ("weighted vw=1.0", SearchFusion::Weighted, 60.0, 1.0, 0.0),
                    ("sat=0.5", SearchFusion::Weighted, 60.0, 0.7, 0.5),
                    ("sat=1", SearchFusion::Weighted, 60.0, 0.7, 1.0),
                    ("sat=2", SearchFusion::Weighted, 60.0, 0.7, 2.0),
                    ("sat=5", SearchFusion::Weighted, 60.0, 0.7, 5.0),
                    ("sat=10", SearchFusion::Weighted, 60.0, 0.7, 10.0),
                    ("sat=15", SearchFusion::Weighted, 60.0, 0.7, 15.0),
                    ("sat=20", SearchFusion::Weighted, 60.0, 0.7, 20.0),
                    ("sat=30", SearchFusion::Weighted, 60.0, 0.7, 30.0),
                    ("sat=50", SearchFusion::Weighted, 60.0, 0.7, 50.0),
                    ("sat=100", SearchFusion::Weighted, 60.0, 0.7, 100.0),
                    ("rrf k=60", SearchFusion::Rrf, 60.0, 0.7, 0.0),
                    ("rrf k=1", SearchFusion::Rrf, 1.0, 0.7, 0.0),
                    ("rrf k=0", SearchFusion::Rrf, 0.0, 0.7, 0.0),
                ] {
                    let config = ab_config(fusion, k, gate, vw, sat);
                    let mut hits = 0usize;
                    let mut mrr = 0.0f64;
                    let mut empty = 0usize;
                    // The metric that actually matches the complaint. Recall of the
                    // *specific* answer file says nothing about the reported failure,
                    // which is a whole SOURCE being scored below an absolute
                    // threshold. `global` is the down-weighted source here, so count
                    // the queries where any global chunk survived into the results.
                    let mut global_visible = 0usize;

                    println!("\n  {label}");
                    for case in &set.cases {
                        let expect: Vec<&str> =
                            case.expect_files.iter().map(String::as_str).collect();
                        let results = hybrid_search(&idx, provider_ref, &case.query, &config)
                            .await
                            .unwrap_or_default();
                        if results.is_empty() {
                            empty += 1;
                        }
                        if results.iter().any(|r| r.source == "global") {
                            global_visible += 1;
                        }
                        match ab_rank_of(&results, &expect) {
                            Some(rank) => {
                                hits += 1;
                                mrr += 1.0 / rank as f64;
                                println!("    rank {rank:>2}  {}", case.query);
                            }
                            None => {
                                let got: Vec<&str> = results
                                    .iter()
                                    .filter_map(|r| std::path::Path::new(&r.path).file_name())
                                    .filter_map(|f| f.to_str())
                                    .collect();
                                println!(
                                    "    MISS    {}  (got: {})",
                                    case.query,
                                    if got.is_empty() {
                                        "nothing".into()
                                    } else {
                                        got.join(", ")
                                    }
                                );
                            }
                        }
                    }
                    summary.push((
                        label,
                        (hits as f64 / n) * 100.0,
                        mrr / n,
                        empty,
                        global_visible,
                    ));
                }

                println!(
                    "\n  {} min_score={gate} — {:<10} {:>9} {:>8} {:>7} {:>14}",
                    set.name, "arm", "recall@6", "MRR", "empty", "global visible"
                );
                for (label, recall, mrr, empty, global_visible) in &summary {
                    println!(
                        "  {:<width$} {label:<16} {recall:>8.0}% {mrr:>8.3} {empty:>7} {global_visible:>11}/{}",
                        "",
                        set.cases.len(),
                        width = set.name.len() + 15
                    );
                }
            }
        }
        println!();
    }
}
