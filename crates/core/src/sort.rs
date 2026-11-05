//! Two small sorts for code that reaches the browser. The standard library's
//! weigh 3 to 5 kB of wasm for every type they sort, which came to a tenth of
//! the engine; these weigh a few dozen bytes.

/// Stable, for a short list or one nearly in order: quadratic otherwise.
pub fn insertion_sort_by<T>(v: &mut [T], mut less: impl FnMut(&T, &T) -> bool) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && less(&v[j], &v[j - 1]) {
            v.swap(j, j - 1);
            j -= 1;
        }
    }
}

/// Any list, in `n log n`. Not stable: `less` must break the ties that matter.
pub fn heap_sort_by<T>(v: &mut [T], mut less: impl FnMut(&T, &T) -> bool) {
    let n = v.len();
    for start in (0..n / 2).rev() {
        sift_down(v, start, n, &mut less);
    }
    for end in (1..n).rev() {
        v.swap(0, end);
        sift_down(v, 0, end, &mut less);
    }
}

fn sift_down<T>(v: &mut [T], mut root: usize, end: usize, less: &mut impl FnMut(&T, &T) -> bool) {
    loop {
        let mut child = 2 * root + 1;
        if child >= end {
            return;
        }
        if child + 1 < end && less(&v[child], &v[child + 1]) {
            child += 1;
        }
        if !less(&v[root], &v[child]) {
            return;
        }
        v.swap(root, child);
        root = child;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed scramble, with repeats.
    fn scrambled(n: usize) -> Vec<(u32, usize)> {
        let mut x = 12345u32;
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                ((x >> 16) % 50, i)
            })
            .collect()
    }

    #[test]
    fn both_agree_with_the_standard_sort() {
        for n in [0, 1, 2, 3, 10, 257, 2000] {
            let mut want = scrambled(n);
            want.sort();
            let mut heap = scrambled(n);
            heap_sort_by(&mut heap, |a, b| a < b);
            assert_eq!(heap, want, "heap, {n}");
            let mut insertion = scrambled(n);
            insertion_sort_by(&mut insertion, |a, b| a < b);
            assert_eq!(insertion, want, "insertion, {n}");
        }
    }

    #[test]
    fn insertion_keeps_equal_items_in_order() {
        let mut v = scrambled(500);
        insertion_sort_by(&mut v, |a, b| a.0 < b.0);
        assert!(v.windows(2).all(|w| w[0].0 < w[1].0 || (w[0].0 == w[1].0 && w[0].1 < w[1].1)));
    }
}
