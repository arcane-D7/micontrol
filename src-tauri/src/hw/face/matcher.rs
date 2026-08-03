//! Face embedding matching: cosine similarity + anti-misrouting margin.
//!
//! Port of `face_hello/matcher.py` from `everglow01/Windows-Face-Hello`.
//! ArcFace normed embeddings are L2-normalized, so cosine similarity == dot
//! product. `best_match_with_margin` prevents unlocking the wrong account in
//! multi-user setups by requiring a minimum gap between the best match and
//! the most-similar *different* profile.

/// Cosine similarity between two L2-normalized (or arbitrary) vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        let x = a[i] as f64;
        let y = b[i] as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())) as f32
}

/// Result of a gallery match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchResult {
    /// Index into the gallery (usize::MAX when gallery empty / no match).
    pub index: usize,
    /// Best cosine similarity.
    pub similarity: f32,
    /// Best − best-different-profile similarity (f32::INFINITY when only one
    /// profile exists — no ambiguity possible).
    pub margin: f32,
}

/// Best match over a flat gallery, ignoring profile grouping.
/// Returns `MatchResult::EMPTY` when the gallery is empty.
impl MatchResult {
    pub const EMPTY: MatchResult = MatchResult {
        index: usize::MAX,
        similarity: 0.0,
        margin: 0.0,
    };
}

/// `names[i]` is the profile name for `gallery[i]`. Same-name templates are
/// not competitors. Returns the best index, similarity, and margin.
pub fn best_match_with_margin(
    probe: &[f32],
    gallery: &[Vec<f32>],
    names: &[String],
) -> MatchResult {
    if gallery.is_empty() || names.len() != gallery.len() {
        return MatchResult::EMPTY;
    }
    let mut sims = Vec::with_capacity(gallery.len());
    for g in gallery {
        sims.push(cosine_similarity(probe, g));
    }
    let mut idx = 0usize;
    let mut best = sims[0];
    for (i, &s) in sims.iter().enumerate().skip(1) {
        if s > best {
            best = s;
            idx = i;
        }
    }
    // Margin vs the most-similar different profile.
    let mut rival_max = f32::MIN;
    let mut has_rival = false;
    for (i, &s) in sims.iter().enumerate() {
        if i != idx && names[i] != names[idx] {
            has_rival = true;
            if s > rival_max {
                rival_max = s;
            }
        }
    }
    let margin = if has_rival {
        best - rival_max
    } else {
        f32::INFINITY
    };
    MatchResult {
        index: idx,
        similarity: best,
        margin,
    }
}

/// Simple best-match without margin (single-profile gallery).
pub fn best_match(probe: &[f32], gallery: &[Vec<f32>]) -> MatchResult {
    if gallery.is_empty() {
        return MatchResult::EMPTY;
    }
    let mut idx = 0usize;
    let mut best = cosine_similarity(probe, &gallery[0]);
    for (i, g) in gallery.iter().enumerate().skip(1) {
        let s = cosine_similarity(probe, g);
        if s > best {
            best = s;
            idx = i;
        }
    }
    MatchResult {
        index: idx,
        similarity: best,
        margin: f32::INFINITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(seed: u32, len: usize) -> Vec<f32> {
        // Deterministic pseudo-random normalized vector.
        let mut out = Vec::with_capacity(len);
        let mut x = seed.wrapping_mul(2654435761);
        for _ in 0..len {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            out.push((x % 1000) as f32 / 1000.0);
        }
        out
    }

    fn l2(v: &mut [f32]) {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 0.0 {
            for x in v.iter_mut() {
                *x /= n;
            }
        }
    }

    #[test]
    fn identical_vectors_give_1() {
        let mut a = v(1, 512);
        l2(&mut a);
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn orthogonal_vectors_give_0() {
        let mut a = vec![1.0f32, 0.0];
        let mut b = vec![0.0f32, 1.0];
        l2(&mut a);
        l2(&mut b);
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn empty_gallery_returns_empty() {
        let r = best_match(&vec![1.0; 4], &[]);
        assert_eq!(r, MatchResult::EMPTY);
    }

    #[test]
    fn best_match_picks_most_similar() {
        let mut probe = v(7, 512);
        l2(&mut probe);
        let mut g1 = v(8, 512);
        l2(&mut g1);
        let mut g2 = v(9, 512);
        l2(&mut g2);
        let gallery = vec![g1.clone(), g2.clone()];
        let r = best_match(&probe, &gallery);
        // probe and g1 share the same first RNG component → higher similarity.
        let s1 = cosine_similarity(&probe, &g1);
        let s2 = cosine_similarity(&probe, &g2);
        assert_eq!(r.index, if s1 > s2 { 0 } else { 1 });
        assert!((r.similarity - s1.max(s2)).abs() < 1e-5);
    }

    #[test]
    fn margin_infinite_for_single_profile() {
        let probe = v(1, 16);
        let g = v(2, 16);
        let r = best_match_with_margin(&probe, &[g], &["alice".to_string()]);
        assert!(r.margin.is_infinite());
        assert_eq!(r.index, 0);
    }

    #[test]
    fn margin_small_when_rivals_close() {
        // Two profiles whose embeddings are nearly identical → tiny margin.
        let probe = v(1, 512);
        let g1 = v(2, 512);
        let g2 = v(3, 512); // different from g1
        let r = best_match_with_margin(
            &probe,
            &[g1.clone(), g2.clone()],
            &["alice".into(), "bob".into()],
        );
        let s1 = cosine_similarity(&probe, &g1);
        let s2 = cosine_similarity(&probe, &g2);
        let expected_margin = (s1 - s2).abs();
        assert!((r.margin - expected_margin).abs() < 1e-4);
        assert!(!r.margin.is_infinite());
    }

    #[test]
    fn same_name_templates_not_rivals() {
        // alice has 2 templates; bob has 1. Best is alice[0] (identical to
        // probe). alice[1] is orthogonal to bob — but even if alice[1] were
        // closer to bob, same-name templates must not count as rivals.
        let probe = vec![1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let alice0 = probe.clone(); // identical → best (sim = 1)
        let alice1 = vec![0.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // orthogonal
        let bob = vec![0.5f32, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let gallery = vec![alice0, alice1.clone(), bob.clone()];
        let names = vec!["alice".into(), "alice".into(), "bob".into()];
        let r = best_match_with_margin(&probe, &gallery, &names);
        assert_eq!(r.index, 0);
        assert!((r.similarity - 1.0).abs() < 1e-5);
        // Rival is bob only (alice1 is same-name). bob sim = 0.5/√0.5 ≈ 0.707.
        // margin = 1 - 0.707 ≈ 0.293.
        let expected = 1.0 - cosine_similarity(&probe, &bob);
        assert!((r.margin - expected).abs() < 1e-4);
    }
}
