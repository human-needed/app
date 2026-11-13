//! "2.0.0", "v0.2.0-test.4" -> [2,0,0]. Pre-release suffixes are ignored.

pub fn parse(v: &str) -> [u32; 3] {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
    let mut out = [0u32; 3];
    for (i, part) in core.split('.').take(3).enumerate() {
        out[i] = part.parse().unwrap_or(0);
    }
    out
}

/// True when `candidate` is strictly newer than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("2.1.0", "2.0.9"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("1.9.9", "2.0.0"));
        assert_eq!(parse("v0.2.0-test.4"), [0, 2, 0]);
        assert_eq!(parse("garbage"), [0, 0, 0]);
    }
}
