//! Version compiled into this binary.

/// `0.1.0+42`. The number after `+` increases when a changed tree is compiled.
pub fn label() -> &'static str {
    env!("BEEFILE_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_has_a_build_number() {
        let (version, build) = label().split_once('+').expect(label());
        assert!(!version.is_empty());
        assert!(build.chars().all(|c| c.is_ascii_digit()) && !build.is_empty());
    }
}
