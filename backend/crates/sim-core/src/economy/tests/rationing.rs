use crate::economy::prorata_distribute;

#[test]
fn prorata_exact_division() {
    assert_eq!(prorata_distribute(&[10, 10], 10), vec![5, 5]);
}

#[test]
fn prorata_proportional() {
    assert_eq!(prorata_distribute(&[30, 10], 20), vec![15, 5]);
}

#[test]
fn prorata_leftover_to_largest_remainder_then_index() {
    // total 2 across three equal weights: floors are [0,0,0], 2 leftover units go
    // to the two largest remainders; all remainders equal -> lowest indices win.
    assert_eq!(prorata_distribute(&[1, 1, 1], 2), vec![1, 1, 0]);
}

#[test]
fn prorata_odd_split_is_deterministic() {
    // 1001 across two equal weights -> 501 / 500 (extra unit to index 0).
    assert_eq!(prorata_distribute(&[1000, 1000], 1001), vec![501, 500]);
}

#[test]
fn prorata_total_at_or_above_sum_returns_weights() {
    assert_eq!(prorata_distribute(&[3, 7], 10), vec![3, 7]);
    assert_eq!(prorata_distribute(&[3, 7], 100), vec![3, 7]);
}

#[test]
fn prorata_zero_total_is_zeros() {
    assert_eq!(prorata_distribute(&[5, 5], 0), vec![0, 0]);
}

#[test]
fn prorata_never_exceeds_a_weight() {
    let weights = [2, 2, 2];
    for total in 0..=6 {
        let out = prorata_distribute(&weights, total);
        assert_eq!(out.iter().sum::<i64>(), total.min(6));
        for (o, w) in out.iter().zip(weights.iter()) {
            assert!(*o <= *w, "alloc {o} exceeded weight {w} at total {total}");
        }
    }
}
