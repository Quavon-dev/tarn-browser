pub fn asks(shell_load: bool, link_activated: bool) -> bool {
    shell_load || link_activated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_load_or_a_link_click_in_the_main_frame_asks() {
        assert!(asks(true, false));
        assert!(asks(false, true));
        assert!(asks(true, true));
        assert!(!asks(false, false), "a script's location change, a form or a history step never asks");
    }
}
