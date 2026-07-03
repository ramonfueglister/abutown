//! `winterthur-traffic` binary stub. Real binary logic (shell loop, weight
//! sampling, re-route cadence) lands in a later task; for now this just
//! proves the crate builds and links as a binary.
fn main() {
    println!("winterthur-traffic {}", env!("CARGO_PKG_VERSION"));
}
