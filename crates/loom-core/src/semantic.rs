//! Deterministic, local-only embeddings for the disposable semantic-index contract.
//!
//! This provider is intentionally a contract baseline rather than a quality claim. It uses stable
//! BLAKE3 token hashing, fixed dimensions, and L2 normalization so a rebuild can be compared bit
//! for bit without downloading a model or making canonical records depend on model availability.

use crate::domain::{SemanticIndexConfig, SemanticProviderMeasurement};
use std::time::Instant;

const MAX_DIMENSION: u32 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkProvider {
    TokenHash,
    CharacterHash,
    TokenCount,
}

impl BenchmarkProvider {
    pub(crate) const ALL: [Self; 3] = [Self::TokenHash, Self::CharacterHash, Self::TokenCount];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::TokenHash => "loom.hash-embedding",
            Self::CharacterHash => "loom.char-embedding",
            Self::TokenCount => "loom.count-embedding",
        }
    }

    pub(crate) fn model_id(self) -> &'static str {
        match self {
            Self::TokenHash => "hashed-tokens-v1",
            Self::CharacterHash => "hashed-char-ngrams-v1",
            Self::TokenCount => "token-counts-v1",
        }
    }
}

/// The default replaceable provider used by the semantic index.
#[derive(Debug, Clone, Default)]
pub(crate) struct HashEmbeddingProvider {
    config: SemanticIndexConfig,
}

impl HashEmbeddingProvider {
    pub(crate) fn config(&self) -> &SemanticIndexConfig {
        &self.config
    }

    pub(crate) fn embed(&self, text: &str) -> Vec<f32> {
        embed_with_provider(
            text,
            self.config.dimension as usize,
            BenchmarkProvider::TokenHash,
        )
    }
}

pub(crate) fn embed_with_provider(
    text: &str,
    dimension: usize,
    provider: BenchmarkProvider,
) -> Vec<f32> {
    let dimension = dimension.clamp(1, MAX_DIMENSION as usize);
    let mut values = vec![0.0_f32; dimension];
    match provider {
        BenchmarkProvider::TokenHash => {
            let tokens = tokens(text);
            for (index, token) in tokens.iter().enumerate() {
                add_hashed(&mut values, token.as_bytes(), 1.0);
                if let Some(next) = tokens.get(index + 1) {
                    let mut bigram = Vec::with_capacity(token.len() + next.len() + 1);
                    bigram.extend_from_slice(token.as_bytes());
                    bigram.push(b' ');
                    bigram.extend_from_slice(next.as_bytes());
                    add_hashed(&mut values, &bigram, 0.5);
                }
            }
        }
        BenchmarkProvider::CharacterHash => {
            let characters = text.chars().collect::<Vec<_>>();
            for window in characters.windows(3) {
                let mut ngram = String::with_capacity(window.len());
                for character in window {
                    ngram.push(character.to_ascii_lowercase());
                }
                add_hashed(&mut values, ngram.as_bytes(), 1.0);
            }
        }
        BenchmarkProvider::TokenCount => {
            for token in tokens(text) {
                let digest = blake3::hash(token.as_bytes());
                let bucket = bucket(&digest, dimension);
                values[bucket] += 1.0;
            }
        }
    }
    normalize_l2(&mut values);
    values
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn add_hashed(values: &mut [f32], bytes: &[u8], weight: f32) {
    let digest = blake3::hash(bytes);
    let bucket = bucket(&digest, values.len());
    let sign = if digest.as_bytes()[8] & 1 == 0 {
        1.0
    } else {
        -1.0
    };
    values[bucket] += sign * weight;
}

fn bucket(digest: &blake3::Hash, dimension: usize) -> usize {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    usize::try_from(u64::from_le_bytes(bytes) % dimension as u64).unwrap_or(0)
}

fn normalize_l2(values: &mut [f32]) {
    let norm = values
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return;
    }
    for value in values {
        *value = (f64::from(*value) / norm) as f32;
    }
}

pub(crate) fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(vector));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub(crate) fn decode_vector(bytes: &[u8], dimension: u32) -> Option<Vec<f32>> {
    let expected = usize::try_from(dimension)
        .ok()?
        .checked_mul(std::mem::size_of::<f32>())?;
    if bytes.len() != expected {
        return None;
    }
    let mut vector = Vec::with_capacity(expected / std::mem::size_of::<f32>());
    let (chunks, remainder) = bytes.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for chunk in chunks {
        let value = f32::from_le_bytes(*chunk);
        if !value.is_finite() {
            return None;
        }
        vector.push(value);
    }
    Some(vector)
}

pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f64> {
    if left.len() != right.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (&left, &right) in left.iter().zip(right) {
        if !left.is_finite() || !right.is_finite() {
            return None;
        }
        dot += f64::from(left) * f64::from(right);
        left_norm += f64::from(left) * f64::from(left);
        right_norm += f64::from(right) * f64::from(right);
    }
    let denominator = (left_norm * right_norm).sqrt();
    (denominator > f64::EPSILON).then_some(dot / denominator)
}

pub(crate) fn measure_providers(
    passages: &[String],
    dimension: u32,
) -> Vec<SemanticProviderMeasurement> {
    let mut measurements = Vec::with_capacity(BenchmarkProvider::ALL.len());
    for provider in BenchmarkProvider::ALL {
        let started = Instant::now();
        let mut vector_bytes = 0_u64;
        for passage in passages {
            let vector = embed_with_provider(passage, dimension as usize, provider);
            vector_bytes = vector_bytes.saturating_add(encode_vector(&vector).len() as u64);
        }
        measurements.push(SemanticProviderMeasurement {
            provider_id: provider.id().into(),
            model_id: provider.model_id().into(),
            dimension,
            sample_count: passages.len() as u64,
            vector_bytes,
            elapsed_micros: started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        });
    }
    measurements
}

#[cfg(test)]
mod tests {
    use super::{
        cosine_similarity, decode_vector, embed_with_provider, encode_vector, BenchmarkProvider,
    };

    #[test]
    fn default_embedding_is_deterministic_normalized_and_bounded() {
        let first =
            embed_with_provider("SQLite retry anomalies", 128, BenchmarkProvider::TokenHash);
        let second =
            embed_with_provider("SQLite retry anomalies", 128, BenchmarkProvider::TokenHash);
        assert_eq!(first, second);
        assert_eq!(first.len(), 128);
        let norm = first
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum::<f64>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
        assert!(first.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn vector_encoding_round_trips_and_rejects_wrong_dimensions() {
        let vector = vec![0.25_f32, -0.5, 1.0];
        let bytes = encode_vector(&vector);
        assert_eq!(decode_vector(&bytes, 3), Some(vector));
        assert_eq!(decode_vector(&bytes, 2), None);
        assert_eq!(decode_vector(&[0, 1, 2], 3), None);
    }

    #[test]
    fn candidate_provider_variants_are_finite_and_comparable() {
        let query = embed_with_provider("database retry", 64, BenchmarkProvider::TokenHash);
        for provider in BenchmarkProvider::ALL {
            let candidate = embed_with_provider("database retry anomalies", 64, provider);
            assert!(candidate.iter().all(|value| value.is_finite()));
            assert!(cosine_similarity(&query, &candidate).is_some());
        }
    }
}
