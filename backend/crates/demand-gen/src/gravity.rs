//! Doubly-constrained gravity model (entropy-maximizing form; Wilson, 1971)
//! balanced with Furness iterations (Ortúzar & Willumsen, 2011, ch. 5):
//!   T_ij = A_i O_i B_j D_j f(c_ij),  f(c) = exp(-c/λ)
//! The caller precomputes the deterrence matrix `f` (row-major, one row per
//! origin) and passes production/attraction vectors with equal sums.

/// Result of a Furness balancing run.
#[derive(Debug)]
pub struct FurnessResult {
    /// Row-major trip matrix, `o.len() * d.len()` entries.
    pub t: Vec<f64>,
    /// Iterations actually run.
    pub iters: usize,
    /// Max relative row/column-sum error at exit.
    pub max_rel_err: f64,
}

/// Balance `T_ij = a_i b_j f_ij` so that row sums match `o` and column sums
/// match `d`, iterating until the max relative error drops below `tol` or
/// `max_iter` is reached. Requires `sum(o) ≈ sum(d)` (scale `d` beforehand)
/// and at least one positive `f` entry per nonzero row/column.
///
/// # Panics
/// Panics on dimension mismatch or if `sum(o)` and `sum(d)` differ by more
/// than 0.1 % — that is a caller bug (the model scales D to O), not data.
pub fn furness(o: &[f64], d: &[f64], f: &[f64], tol: f64, max_iter: usize) -> FurnessResult {
    let (rows, cols) = (o.len(), d.len());
    assert_eq!(f.len(), rows * cols, "deterrence matrix dimension mismatch");
    let (so, sd): (f64, f64) = (o.iter().sum(), d.iter().sum());
    assert!(
        so > 0.0 && ((so - sd) / so).abs() < 1e-3,
        "sum(o)={so} and sum(d)={sd} must match (scale d to o first)"
    );

    // T_ij = a_i * b_j * f_ij; alternate row scalings (a) and column
    // scalings (b) until both marginals match within tol.
    let mut a = vec![1.0f64; rows];
    let mut b = vec![1.0f64; cols];
    let mut iters = 0usize;
    let mut max_rel_err = f64::INFINITY;
    while iters < max_iter {
        iters += 1;
        // a_i = O_i / Σ_j b_j f_ij
        for i in 0..rows {
            let s: f64 = (0..cols).map(|j| b[j] * f[i * cols + j]).sum();
            a[i] = if s > 0.0 { o[i] / s } else { 0.0 };
        }
        // b_j = D_j / Σ_i a_i f_ij
        for j in 0..cols {
            let s: f64 = (0..rows).map(|i| a[i] * f[i * cols + j]).sum();
            b[j] = if s > 0.0 { d[j] / s } else { 0.0 };
        }
        // convergence: max relative marginal error
        max_rel_err = 0.0f64;
        for i in 0..rows {
            if o[i] <= 0.0 {
                continue;
            }
            let row: f64 = (0..cols).map(|j| a[i] * b[j] * f[i * cols + j]).sum();
            max_rel_err = max_rel_err.max((row - o[i]).abs() / o[i]);
        }
        for j in 0..cols {
            if d[j] <= 0.0 {
                continue;
            }
            let col: f64 = (0..rows).map(|i| a[i] * b[j] * f[i * cols + j]).sum();
            max_rel_err = max_rel_err.max((col - d[j]).abs() / d[j]);
        }
        if max_rel_err < tol {
            break;
        }
    }

    let mut t = vec![0.0f64; rows * cols];
    for i in 0..rows {
        for j in 0..cols {
            t[i * cols + j] = a[i] * b[j] * f[i * cols + j];
        }
    }
    FurnessResult {
        t,
        iters,
        max_rel_err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furness_converges_on_3x3_toy() {
        let o = [10.0, 20.0, 30.0];
        // column targets, same total (60)
        let d = [15.0, 15.0, 30.0];
        // asymmetric deterrence matrix, all positive
        let f = [
            1.0, 0.5, 0.2, //
            0.4, 1.0, 0.6, //
            0.1, 0.7, 1.0,
        ];
        let res = furness(&o, &d, &f, 1e-3, 100);
        assert!(res.max_rel_err < 1e-3, "did not converge: {res:?}");
        for (i, &oi) in o.iter().enumerate() {
            let row: f64 = (0..3).map(|j| res.t[i * 3 + j]).sum();
            assert!((row - oi).abs() / oi < 1e-3, "row {i} sum {row} != {oi}");
        }
        for (j, &dj) in d.iter().enumerate() {
            let col: f64 = (0..3).map(|i| res.t[i * 3 + j]).sum();
            assert!((col - dj).abs() / dj < 1e-3, "col {j} sum {col} != {dj}");
        }
        // all cells nonnegative
        assert!(res.t.iter().all(|&x| x >= 0.0));
    }

    #[test]
    fn furness_is_deterministic() {
        let o = [5.0, 7.0];
        let d = [6.0, 6.0];
        let f = [1.0, 0.3, 0.3, 1.0];
        let a = furness(&o, &d, &f, 1e-4, 100);
        let b = furness(&o, &d, &f, 1e-4, 100);
        assert_eq!(a.t, b.t);
    }
}
