/// Interpolates amplitude toward full volume with cubic-bezier(0, 1.03, 0.53, 0.39).
/// Solve the curve's time axis before reading its amplitude, as CSS timing functions do.
pub(super) fn level(from: f32, progress: f32) -> f32 {
    let t = ui::motion::cubic_bezier(progress, 0., 1.03, 0.53, 0.39);
    (from + (1. - from) * t).clamp(from.min(1.), from.max(1.))
}

#[cfg(test)]
mod tests {
    use super::level;

    #[test]
    fn preserves_endpoints_and_stays_monotonic() {
        for from in [0., 0.37, 1.] {
            assert_eq!(level(from, 0.), from);
            assert_eq!(level(from, 1.), 1.);
            let mut previous = from;
            for step in 0..=1000 {
                let gain = level(from, step as f32 / 1000.);
                assert!((from..=1.).contains(&gain));
                assert!(gain >= previous);
                previous = gain;
            }
        }
    }

    #[test]
    fn solves_time_instead_of_using_time_as_its_parameter() {
        for (time, expected) in [(0.015_31, 0.261_82), (0.323_75, 0.657_5)] {
            assert!((level(0., time) - expected).abs() < 1e-6);
        }
        let parameter: f32 = 0.0001;
        let time = 3. * (1. - parameter) * parameter * parameter * 0.53 + parameter.powi(3);
        let expected = 3. * (1. - parameter).powi(2) * parameter * 1.03
            + 3. * (1. - parameter) * parameter.powi(2) * 0.39
            + parameter.powi(3);
        assert!((level(0., time) - expected).abs() < 1e-6);
    }
}
