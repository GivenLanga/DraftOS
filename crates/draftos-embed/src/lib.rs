//! Embedding providers. The trait is the contract; implementations are
//! swappable. Only this crate (and draftos-models later) may touch the
//! network — and the default provider is fully offline.
//!
//! Current default: `HashEmbedder`, a deterministic feature-hashing embedder
//! (word unigrams + char trigrams → fixed dims, L2-normalized). It is fully
//! offline and fast; retrieval quality leans on FTS5/BM25 until the ONNX
//! `fastembed` provider lands behind a cargo feature (see docs/DECISIONS.md).
//! Because bundles record their embed model in manifest.json, upgrading the
//! embedder never silently invalidates an existing source.

use anyhow::Result;

pub trait EmbeddingProvider: Send + Sync {
    /// Stable identifier recorded in each bundle's manifest ("hash-v1@256").
    fn id(&self) -> String;
    fn dims(&self) -> usize;
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed(&[text])?.remove(0))
    }
}

pub struct HashEmbedder {
    dims: usize,
}

impl HashEmbedder {
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }

    pub fn default_dims() -> usize {
        256
    }

    fn accumulate(&self, vector: &mut [f32], token: &str, weight: f32) {
        let h = fnv1a(token.as_bytes());
        let idx = (h % self.dims as u64) as usize;
        // Second hash decides sign, which spreads collisions symmetrically.
        let sign = if (h >> 32) & 1 == 0 { 1.0 } else { -1.0 };
        vector[idx] += sign * weight;
    }
}

impl EmbeddingProvider for HashEmbedder {
    fn id(&self) -> String {
        format!("hash-v1@{}", self.dims)
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_text(t)).collect())
    }
}

impl HashEmbedder {
    fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dims];
        let normalized: String = text
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect();

        for word in normalized.split_whitespace() {
            self.accumulate(&mut v, word, 1.0);
            // Char trigrams catch morphology ("terminate"/"termination").
            let chars: Vec<char> = word.chars().collect();
            if chars.len() > 3 {
                for w in chars.windows(3) {
                    let tri: String = w.iter().collect();
                    self.accumulate(&mut v, &tri, 0.5);
                }
            }
        }

        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_normalized() {
        let e = HashEmbedder::new(256);
        let a = e.embed_one("The Seller may terminate this Agreement").unwrap();
        let b = e.embed_one("The Seller may terminate this Agreement").unwrap();
        assert_eq!(a, b);
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn related_texts_closer_than_unrelated() {
        let e = HashEmbedder::new(256);
        let term1 = e.embed_one("termination of the agreement on breach").unwrap();
        let term2 = e.embed_one("terminate the agreement for material breach").unwrap();
        let other = e.embed_one("the purchase price is payable in cash").unwrap();
        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(a, b)| a * b).sum::<f32>();
        assert!(dot(&term1, &term2) > dot(&term1, &other));
    }
}
