//! Group near-identical claims without discarding evidence or permissions.
//!
//! Identities are free, so copies of one response can be submitted under many
//! keys. Responses whose text has word-shingle Jaccard similarity of at least
//! [`THRESHOLD`] share a group for model weighting and merge counts. All
//! responses remain in synthesis and privacy checks. Paraphrased floods aren't
//! caught; that's disclosed.

use std::collections::BTreeSet;

pub const THRESHOLD: f64 = 0.8;
const SHINGLE: usize = 4;

pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn shingles(text: &str) -> BTreeSet<Vec<String>> {
    let w = words(text);
    if w.len() < SHINGLE {
        return std::iter::once(w).collect();
    }
    w.windows(SHINGLE).map(<[String]>::to_vec).collect()
}

fn jaccard(a: &BTreeSet<Vec<String>>, b: &BTreeSet<Vec<String>>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 1.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// Representative index for every response. Grouping never discards the
/// evidence or publication permissions attached to an individual response.
pub fn groups(texts: &[String]) -> Vec<usize> {
    let sets: Vec<_> = texts.iter().map(|t| shingles(t)).collect();
    let mut kept: Vec<usize> = Vec::new();
    let mut groups = Vec::with_capacity(texts.len());
    for (i, s) in sets.iter().enumerate() {
        let representative = kept
            .iter()
            .copied()
            .find(|&k| jaccard(&sets[k], s) >= THRESHOLD)
            .unwrap_or_else(|| {
                kept.push(i);
                i
            });
        groups.push(representative);
    }
    groups
}

/// Indices of the first response in each group, used by the grouping tests.
#[cfg(test)]
fn representatives(texts: &[String]) -> Vec<usize> {
    groups(texts)
        .into_iter()
        .enumerate()
        .filter_map(|(i, k)| (i == k).then_some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_fold_and_distinct_stay() {
        let texts = vec![
            "The agent paid the merchant twice after a fake failure message.".to_string(),
            "the agent paid the merchant twice, after a fake failure message!".to_string(),
            "Allowance expiry should match the invoice due date.".to_string(),
        ];
        assert_eq!(representatives(&texts), vec![0, 2]);
    }

    #[test]
    fn short_texts_compare_whole() {
        let texts = vec!["yes".to_string(), "Yes.".to_string(), "no".to_string()];
        assert_eq!(representatives(&texts), vec![0, 2]);
    }
}
