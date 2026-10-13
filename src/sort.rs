//! Stable sorting with little code per element type.
//!
//! std's sorts (driftsort, ipnsort) are fast and big: 3 to 4 KB for every
//! element type and comparison, about 70 KB over the app's sorts, which
//! are of a few to a few thousand items. Here short slices take an
//! insertion sort; longer ones sort a list of indices with one shared,
//! non-generic merge sort (comparing through `dyn`), then move the
//! elements into place with swaps.

use std::cmp::Ordering;

/// Slices up to this long take the insertion sort.
const SHORT: usize = 20;

/// Sort `v` (stable).
pub fn sort<T: Ord>(v: &mut [T]) {
    by_less(v, |a, b| a < b);
}

/// Sort `v` by `cmp` (stable).
pub fn by<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> Ordering) {
    by_less(v, |a, b| cmp(a, b) == Ordering::Less);
}

/// Sort `v` by `key` (stable).
pub fn by_key<T, K: Ord>(v: &mut [T], mut key: impl FnMut(&T) -> K) {
    by_less(v, |a, b| key(a) < key(b));
}

fn by_less<T>(v: &mut [T], mut less: impl FnMut(&T, &T) -> bool) {
    let n = v.len();
    if n <= SHORT {
        for i in 1..n {
            let mut j = i;
            while j > 0 && less(&v[j], &v[j - 1]) {
                v.swap(j, j - 1);
                j -= 1;
            }
        }
        return;
    }
    let order = {
        let v = &*v;
        sorted_indices(n, &mut |a, b| less(&v[a], &v[b]))
    };
    // `order[k]` is the element for place k. `to[j]`: the place of the
    // element now at j; each swap puts one element where it belongs.
    let mut to = vec![0usize; n];
    for (k, &j) in order.iter().enumerate() {
        to[j] = k;
    }
    for i in 0..n {
        while to[i] != i {
            let d = to[i];
            v.swap(i, d);
            to.swap(i, d);
        }
    }
}

/// The indices `0..n` in sorted order (stable): runs of [`SHORT`] by
/// insertion, then merged bottom-up.
fn sorted_indices(n: usize, less: &mut dyn FnMut(usize, usize) -> bool) -> Vec<usize> {
    let mut a: Vec<usize> = (0..n).collect();
    for run in a.chunks_mut(SHORT) {
        for i in 1..run.len() {
            let mut j = i;
            while j > 0 && less(run[j], run[j - 1]) {
                run.swap(j, j - 1);
                j -= 1;
            }
        }
    }
    let mut b = vec![0usize; n];
    let mut w = SHORT;
    while w < n {
        let mut s = 0;
        while s < n {
            let (m, e) = ((s + w).min(n), (s + 2 * w).min(n));
            let (mut i, mut j, mut k) = (s, m, s);
            while i < m && j < e {
                // Equal: the left one first (stable).
                if less(a[j], a[i]) {
                    b[k] = a[j];
                    j += 1;
                } else {
                    b[k] = a[i];
                    i += 1;
                }
                k += 1;
            }
            b[k..k + m - i].copy_from_slice(&a[i..m]);
            k += m - i;
            b[k..k + e - j].copy_from_slice(&a[j..e]);
            s = e;
        }
        std::mem::swap(&mut a, &mut b);
        w *= 2;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_std_and_stays_stable() {
        let mut s = 0x1234_5678u64;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for n in [0, 1, 2, 3, 19, 20, 21, 39, 40, 41, 100, 1000, 4097] {
            for range in [3u64, 1000, u64::MAX] {
                // (key, original index): stability shows in the index order.
                let v: Vec<(u64, usize)> = (0..n).map(|i| (rng() % range, i)).collect();
                let mut want = v.clone();
                want.sort_by_key(|p| p.0);
                let mut got = v.clone();
                by_key(&mut got, |p| p.0);
                assert_eq!(got, want, "by_key n {n} range {range}");
                let mut got = v.clone();
                by(&mut got, |a, b| b.0.cmp(&a.0));
                let mut want_rev = v.clone();
                want_rev.sort_by_key(|p| std::cmp::Reverse(p.0));
                assert_eq!(got, want_rev, "by n {n}");
                let mut got: Vec<u64> = v.iter().map(|p| p.0).collect();
                sort(&mut got);
                assert!(got.windows(2).all(|w| w[0] <= w[1]));
            }
        }
        let mut strs = vec!["b".to_string(), "a".into(), "c".into(), "a".into()];
        sort(&mut strs);
        assert_eq!(strs, ["a", "a", "b", "c"]);
    }
}
