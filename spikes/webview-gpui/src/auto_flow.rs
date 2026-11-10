// Each wait is relative to the preceding transition; never skip lifecycle stages.
pub fn next_stage(stage: u8, seconds: f64, ready: bool) -> u8 {
    let advance = match stage {
        0 => seconds >= 1.0,
        1 | 6 => ready,
        2 | 3 | 4 | 7 => seconds >= 3.0,
        5 => seconds >= 0.2,
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

// The transparent aperture is allowed only while the visible child covers it.
pub fn paint_background(transparent: bool, ready: bool, hidden: bool) -> bool {
    !transparent || !ready || hidden
}
#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn no_hole_while_loading_closed_or_hidden() {
        assert!(paint_background(true, false, false));
        assert!(paint_background(true, true, true));
        assert!(!paint_background(true, true, false));
        assert!(paint_background(false, true, false));
    }
    #[test]
    fn reopen_after_200ms_without_skipping_ready_wait() {
        assert_eq!(next_stage(5, 0.199, false), 5);
        assert_eq!(next_stage(5, 0.2, false), 6);
        assert_eq!(next_stage(6, 8.0, false), 6);
    }
}
