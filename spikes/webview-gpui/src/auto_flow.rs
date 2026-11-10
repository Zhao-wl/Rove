// Each wait is relative to the preceding transition; never skip lifecycle stages.
pub fn next_stage(stage: u8, seconds: f64, ready: bool) -> u8 {
    let advance = match stage {
        0 => seconds >= 1.0,
        1 | 6 => ready,
        2 | 3 | 4 | 7 => seconds >= 3.0,
        5 => seconds >= 2.0,
        _ => false,
    };
    stage + u8::from(advance)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_creation_never_closes_before_ready() {
        for stage in [1, 6] {
            assert_eq!(next_stage(stage, 19.0, false), stage);
            assert_eq!(next_stage(stage, 19.0, true), stage + 1);
        }
    }
    #[test]
    fn delayed_frame_cannot_skip_stages() {
        for stage in 0..8 {
            assert_eq!(next_stage(stage, 100.0, true), stage + 1);
        }
    }
    #[test]
    fn rebuilt_page_gets_observation_time() {
        assert_eq!(next_stage(7, 2.9, true), 7);
        assert_eq!(next_stage(7, 3.0, true), 8);
    }
}
