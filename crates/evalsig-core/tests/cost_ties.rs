//! Expected costs that are equal in exact arithmetic tie, and the lower threshold wins
//! (metrics spec §4), whatever the last digits of the floating-point costs say.

use evalsig_core::cost;

#[test]
fn an_exact_tie_goes_to_the_lower_threshold() {
    // Thresholds 2 and 3 both cost exactly 1/10.
    let m = cost::ratio_matrix(1.0);
    let (t, _) = cost::optimal_threshold(
        &[1.0, 4.0, 10.0, 30.0, 55.0],
        &[60.0, 25.0, 10.0, 4.0, 1.0],
        &m,
        0.5,
    )
    .unwrap();
    assert_eq!(t, 2);
    // The costs are exactly 1/2, 3/8 and 3/8.
    let m = cost::ratio_matrix(2.0);
    let (t, _) = cost::optimal_threshold(
        &[10.0, 20.0, 30.0, 40.0],
        &[40.0, 30.0, 20.0, 10.0],
        &m,
        0.25,
    )
    .unwrap();
    assert_eq!(t, 2);
}
