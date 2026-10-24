//! In-process sequence diff (Myers, linear space) used for the gutter, the side-by-side
//! viewer and inline word changes. Elements are interned first, elements that only exist on
//! one side are set aside (they can never match) and the common prefix/suffix is skipped, so
//! typical edits of large files cost little more than hashing their lines.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;

/// One block of differences: `old` elements replaced by `new` ones. One side may be empty
/// (pure insertion or deletion); `old.start`/`new.start` still say where the block sits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Change {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// Default cap on the edit cost explored per split; past it a region is reported as one
/// replaced block instead of searching on (only reached by pathological inputs).
pub const DEFAULT_COST_LIMIT: usize = 2048;

/// The blocks that turn `old` into `new`, in order, using a minimal edit script.
pub fn diff<T: Hash + Eq>(old: &[T], new: &[T]) -> Vec<Change> {
    diff_with_limit(old, new, DEFAULT_COST_LIMIT)
}

/// Like [`diff`] with an explicit cost cap (`usize::MAX` = always minimal).
pub fn diff_with_limit<T: Hash + Eq>(old: &[T], new: &[T], limit: usize) -> Vec<Change> {
    changes_from_matches(&matches(old, new, limit), old.len(), new.len())
}

/// The matched element pairs `(old index, new index)` of a minimal diff, increasing.
pub fn matches<T: Hash + Eq>(old: &[T], new: &[T], limit: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    // Common prefix and suffix need no interning.
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let rest = (old.len() - prefix).min(new.len() - prefix);
    let suffix = old[old.len() - rest..].iter().rev().zip(new[new.len() - rest..].iter().rev()).take_while(|(a, b)| a == b).count();
    out.extend((0..prefix).map(|i| (i, i)));
    let (a, b) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);
    if !a.is_empty() && !b.is_empty() {
        let mut ids: HashMap<&T, u32> = HashMap::with_capacity(a.len() + b.len());
        let ia: Vec<u32> = a.iter().map(|x| intern(&mut ids, x)).collect();
        let ib: Vec<u32> = b.iter().map(|x| intern(&mut ids, x)).collect();
        let n_ids = ids.len();
        let mut in_a = vec![false; n_ids];
        let mut in_b = vec![false; n_ids];
        ia.iter().for_each(|&i| in_a[i as usize] = true);
        ib.iter().for_each(|&i| in_b[i as usize] = true);
        // Elements present on one side only can never be part of a common subsequence.
        let (fa, ma): (Vec<u32>, Vec<usize>) = ia.iter().enumerate().filter(|(_, id)| in_b[**id as usize]).map(|(i, id)| (*id, i)).unzip();
        let (fb, mb): (Vec<u32>, Vec<usize>) = ib.iter().enumerate().filter(|(_, id)| in_a[**id as usize]).map(|(i, id)| (*id, i)).unzip();
        if !fa.is_empty() && !fb.is_empty() {
            let size = (fa.len() + fb.len()) / 2 + 2;
            let mut vf = V::new(size);
            let mut vb = V::new(size);
            let mut inner = Vec::new();
            conquer(&fa, 0, fa.len(), &fb, 0, fb.len(), &mut vf, &mut vb, limit, &mut inner);
            out.extend(inner.into_iter().map(|(i, j)| (ma[i] + prefix, mb[j] + prefix)));
        }
    }
    out.extend((0..suffix).map(|i| (old.len() - suffix + i, new.len() - suffix + i)));
    out
}

fn intern<'a, T: Hash + Eq>(ids: &mut HashMap<&'a T, u32>, x: &'a T) -> u32 {
    let next = ids.len() as u32;
    *ids.entry(x).or_insert(next)
}

/// Turns increasing matched pairs into the blocks between them.
pub fn changes_from_matches(matches: &[(usize, usize)], old_len: usize, new_len: usize) -> Vec<Change> {
    let mut out = Vec::new();
    let (mut pa, mut pb) = (0, 0);
    for &(i, j) in matches {
        if i > pa || j > pb {
            out.push(Change { old: pa..i, new: pb..j });
        }
        pa = i + 1;
        pb = j + 1;
    }
    if pa < old_len || pb < new_len {
        out.push(Change { old: pa..old_len, new: pb..new_len });
    }
    out
}

/// A Myers V array indexed by diagonal `k` in `-size..=size`.
struct V {
    offset: isize,
    v: Vec<usize>,
}

impl V {
    fn new(size: usize) -> V {
        V { offset: size as isize + 1, v: vec![0; 2 * size + 3] }
    }
}

impl std::ops::Index<isize> for V {
    type Output = usize;
    fn index(&self, k: isize) -> &usize {
        &self.v[(k + self.offset) as usize]
    }
}

impl std::ops::IndexMut<isize> for V {
    fn index_mut(&mut self, k: isize) -> &mut usize {
        &mut self.v[(k + self.offset) as usize]
    }
}

#[allow(clippy::too_many_arguments)]
fn conquer(a: &[u32], mut alo: usize, mut ahi: usize, b: &[u32], mut blo: usize, mut bhi: usize, vf: &mut V, vb: &mut V, limit: usize, out: &mut Vec<(usize, usize)>) {
    while alo < ahi && blo < bhi && a[alo] == b[blo] {
        out.push((alo, blo));
        alo += 1;
        blo += 1;
    }
    let end_a = ahi;
    while alo < ahi && blo < bhi && a[ahi - 1] == b[bhi - 1] {
        ahi -= 1;
        bhi -= 1;
    }
    if alo < ahi && blo < bhi {
        if let Some((x, y)) = middle_snake(a, alo, ahi, b, blo, bhi, vf, vb, limit) {
            if (x, y) != (alo, blo) && (x, y) != (ahi, bhi) {
                conquer(a, alo, x, b, blo, y, vf, vb, limit, out);
                conquer(a, x, ahi, b, y, bhi, vf, vb, limit, out);
            }
        }
    }
    out.extend((0..end_a - ahi).map(|i| (ahi + i, bhi + i)));
}

/// A point on an optimal path through the edit graph of `a[alo..ahi]`/`b[blo..bhi]`, found
/// by running the forward and backward searches until they overlap.
#[allow(clippy::too_many_arguments)]
fn middle_snake(a: &[u32], alo: usize, ahi: usize, b: &[u32], blo: usize, bhi: usize, vf: &mut V, vb: &mut V, limit: usize) -> Option<(usize, usize)> {
    let n = ahi - alo;
    let m = bhi - blo;
    let delta = n as isize - m as isize;
    let odd = delta & 1 == 1;
    vf[1] = 0;
    vb[1] = 0;
    let d_max = ((n + m).div_ceil(2) + 1).min(limit) as isize;
    for d in 0..d_max {
        let mut k = d;
        while k >= -d {
            let mut x = if k == -d || (k != d && vf[k - 1] < vf[k + 1]) { vf[k + 1] } else { vf[k - 1] + 1 };
            let mut y = (x as isize - k) as usize;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[alo + x] == b[blo + y] {
                x += 1;
                y += 1;
            }
            vf[k] = x;
            if odd && (k - delta).abs() < d && vf[k] + vb[-(k - delta)] >= n {
                return Some((x0 + alo, y0 + blo));
            }
            k -= 2;
        }
        let mut k = d;
        while k >= -d {
            let mut x = if k == -d || (k != d && vb[k - 1] < vb[k + 1]) { vb[k + 1] } else { vb[k - 1] + 1 };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m && a[ahi - x - 1] == b[bhi - y - 1] {
                x += 1;
                y += 1;
            }
            vb[k] = x;
            if !odd && (k - delta).abs() <= d && vb[k] + vf[-(k - delta)] >= n {
                return Some((n.saturating_sub(x) + alo, m.saturating_sub(y) + blo));
            }
            k -= 2;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply<T: Clone>(old: &[T], new: &[T], changes: &[Change]) -> Vec<T> {
        let mut out = Vec::new();
        let mut at = 0;
        for c in changes {
            out.extend_from_slice(&old[at..c.old.start]);
            out.extend_from_slice(&new[c.new.clone()]);
            at = c.old.end;
        }
        out.extend_from_slice(&old[at..]);
        out
    }

    fn lcs_len(a: &[u8], b: &[u8]) -> usize {
        let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in 0..a.len() {
            for j in 0..b.len() {
                dp[i + 1][j + 1] = if a[i] == b[j] { dp[i][j] + 1 } else { dp[i][j + 1].max(dp[i + 1][j]) };
            }
        }
        dp[a.len()][b.len()]
    }

    #[test]
    fn simple_blocks() {
        let old = ["a", "b", "c", "d"];
        let new = ["a", "x", "c", "d", "e"];
        assert_eq!(diff(&old, &new), vec![Change { old: 1..2, new: 1..2 }, Change { old: 4..4, new: 4..5 }]);
        assert!(diff(&old, &old).is_empty());
        assert_eq!(diff::<&str>(&[], &["a"]), vec![Change { old: 0..0, new: 0..1 }]);
        assert_eq!(diff::<&str>(&["a"], &[]), vec![Change { old: 0..1, new: 0..0 }]);
    }

    proptest::proptest! {
        #[test]
        fn minimal_and_reconstructs(a in proptest::collection::vec(0u8..4, 0..40), b in proptest::collection::vec(0u8..4, 0..40)) {
            let m = matches(&a, &b, usize::MAX);
            proptest::prop_assert_eq!(m.len(), lcs_len(&a, &b));
            for w in m.windows(2) {
                proptest::prop_assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1);
            }
            for &(i, j) in &m {
                proptest::prop_assert_eq!(a[i], b[j]);
            }
            let changes = changes_from_matches(&m, a.len(), b.len());
            proptest::prop_assert_eq!(apply(&a, &b, &changes), b.clone());
        }

        #[test]
        fn limited_diff_still_reconstructs(a in proptest::collection::vec(0u8..3, 0..60), b in proptest::collection::vec(0u8..3, 0..60)) {
            let changes = diff_with_limit(&a, &b, 2);
            proptest::prop_assert_eq!(apply(&a, &b, &changes), b.clone());
        }
    }
}
