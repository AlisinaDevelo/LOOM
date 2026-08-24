//! Deterministic, explainable hybrid rank fusion.
//!
//! This module is intentionally separate from the desktop search path. It provides an
//! evidence-preserving experimental ranker that can be evaluated against the public fixture
//! before a product decision promotes it to the default UI behavior.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    domain::{EvidenceAnchor, EvidenceExcerpt},
    error::{LoomError, Result},
};

/// Candidate evidence assembled from the lexical and semantic retrieval channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridRankInput {
    pub artifact_id: String,
    pub version_id: String,
    pub passage_id: String,
    pub title: String,
    pub media_type: String,
    pub source_uri: String,
    pub content_hash: String,
    pub passage_text: String,
    pub excerpt: EvidenceExcerpt,
    pub anchor: EvidenceAnchor,
    /// Source modification time in nanoseconds, when the source supplied one.
    pub source_modified_ns: Option<i64>,
    pub lexical_rank: Option<u32>,
    pub semantic_rank: Option<u32>,
}

/// Per-signal evidence retained with every hybrid result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridSignalEvidence {
    pub lexical_rank: Option<u32>,
    pub lexical_rrf: f64,
    pub semantic_rank: Option<u32>,
    pub semantic_rrf: f64,
    pub exact_match: bool,
    pub path_token_overlap: f64,
    pub recency_score: f64,
}

/// A source-backed result produced by the experimental hybrid ranker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridSearchHit {
    pub rank: u32,
    pub score: f64,
    pub artifact_id: String,
    pub version_id: String,
    pub passage_id: String,
    pub title: String,
    pub media_type: String,
    pub source_uri: String,
    pub content_hash: String,
    pub excerpt: EvidenceExcerpt,
    pub anchor: EvidenceAnchor,
    pub signals: HybridSignalEvidence,
    pub match_reason: String,
}

/// Versioned weights for the preregistered hybrid experiment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HybridRankConfig {
    pub reciprocal_rank_constant: u32,
    pub lexical_weight: f64,
    pub semantic_weight: f64,
    pub exact_match_weight: f64,
    pub path_weight: f64,
    pub recency_weight: f64,
}

impl Default for HybridRankConfig {
    fn default() -> Self {
        Self {
            reciprocal_rank_constant: 60,
            lexical_weight: 0.45,
            semantic_weight: 0.35,
            exact_match_weight: 0.10,
            path_weight: 0.05,
            recency_weight: 0.05,
        }
    }
}

/// Fuses candidate ranks with weighted reciprocal-rank fusion and bounded metadata signals.
///
/// The order is deterministic: equal scores are resolved by the stable passage ID. Every output
/// retains the original source tuple and an explicit contribution record; no generated text is
/// introduced by this function.
pub fn fuse_hybrid_candidates(
    query: &str,
    inputs: Vec<HybridRankInput>,
    config: &HybridRankConfig,
) -> Result<Vec<HybridSearchHit>> {
    validate_query(query)?;
    validate_config(config)?;

    let recency = recency_scores(&inputs);
    let mut ranked = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            let lexical_rrf = reciprocal_rank(input.lexical_rank, config.reciprocal_rank_constant);
            let semantic_rrf =
                reciprocal_rank(input.semantic_rank, config.reciprocal_rank_constant);
            let exact_match = exact_match(query, &input);
            let path_token_overlap = path_token_overlap(query, &input);
            let recency_score = recency[index];
            let signals = HybridSignalEvidence {
                lexical_rank: input.lexical_rank,
                lexical_rrf,
                semantic_rank: input.semantic_rank,
                semantic_rrf,
                exact_match,
                path_token_overlap,
                recency_score,
            };
            let score = config.lexical_weight * lexical_rrf
                + config.semantic_weight * semantic_rrf
                + config.exact_match_weight * f64::from(exact_match)
                + config.path_weight * path_token_overlap
                + config.recency_weight * recency_score;
            HybridSearchHit {
                rank: 0,
                score,
                artifact_id: input.artifact_id,
                version_id: input.version_id,
                passage_id: input.passage_id,
                title: input.title,
                media_type: input.media_type,
                source_uri: input.source_uri,
                content_hash: input.content_hash,
                excerpt: input.excerpt,
                anchor: input.anchor,
                signals,
                match_reason: "hybrid-rank-v1 weighted reciprocal-rank fusion".into(),
            }
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.passage_id.cmp(&right.passage_id))
    });
    for (index, hit) in ranked.iter_mut().enumerate() {
        hit.rank = index as u32 + 1;
    }
    Ok(ranked)
}

fn validate_query(query: &str) -> Result<()> {
    let normalized = normalize(query);
    if normalized.is_empty() {
        return Err(LoomError::InvalidQuery("query cannot be empty".into()));
    }
    if normalized.chars().count() > 512 {
        return Err(LoomError::InvalidQuery(
            "query exceeds the 512-character limit".into(),
        ));
    }
    Ok(())
}

fn validate_config(config: &HybridRankConfig) -> Result<()> {
    if config.reciprocal_rank_constant == 0 {
        return Err(LoomError::InvalidQuery(
            "hybrid reciprocal-rank constant must be positive".into(),
        ));
    }
    let weights = [
        config.lexical_weight,
        config.semantic_weight,
        config.exact_match_weight,
        config.path_weight,
        config.recency_weight,
    ];
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
        || weights.iter().copied().sum::<f64>() <= 0.0
    {
        return Err(LoomError::InvalidQuery(
            "hybrid weights must be finite, non-negative, and non-zero".into(),
        ));
    }
    Ok(())
}

fn reciprocal_rank(rank: Option<u32>, constant: u32) -> f64 {
    rank.map(|rank| 1.0 / (f64::from(constant) + f64::from(rank.max(1))))
        .unwrap_or(0.0)
}

fn exact_match(query: &str, input: &HybridRankInput) -> bool {
    let query = normalize(query);
    [
        normalize(&input.passage_text),
        normalize(&input.title),
        normalize(&input.source_uri),
    ]
    .iter()
    .any(|value| value.contains(&query))
}

fn path_token_overlap(query: &str, input: &HybridRankInput) -> f64 {
    let query_tokens = unique_tokens(query);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let path_tokens = unique_tokens(&format!("{} {}", input.title, input.source_uri));
    let overlap = query_tokens.intersection(&path_tokens).count();
    overlap as f64 / query_tokens.len() as f64
}

fn recency_scores(inputs: &[HybridRankInput]) -> Vec<f64> {
    let timestamps = inputs
        .iter()
        .filter_map(|input| input.source_modified_ns)
        .collect::<Vec<_>>();
    let (Some(minimum), Some(maximum)) = (timestamps.iter().min(), timestamps.iter().max()) else {
        return vec![0.0; inputs.len()];
    };
    if minimum == maximum {
        return inputs
            .iter()
            .map(|input| input.source_modified_ns.map(|_| 1.0).unwrap_or(0.0))
            .collect();
    }
    let range = (*maximum as i128 - *minimum as i128) as f64;
    inputs
        .iter()
        .map(|input| {
            input
                .source_modified_ns
                .map(|value| ((value as i128 - *minimum as i128) as f64 / range).clamp(0.0, 1.0))
                .unwrap_or(0.0)
        })
        .collect()
}

fn unique_tokens(value: &str) -> BTreeSet<String> {
    normalize(value)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn normalize(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
        } else if !normalized.ends_with(' ') {
            normalized.push(' ');
        }
    }
    normalized.trim().to_string()
}
