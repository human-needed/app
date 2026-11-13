//! The puck only sees 8 lowercase hex ids (`hn-usb/2` §3). The app hashes `agent` + `session`.

/// FNV-1a 32-bit of `agent \x1f session`, as 8 lowercase hex characters. Stable across runs.
pub fn puck_id(agent: &str, session: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in agent.bytes().chain(std::iter::once(0x1f)).chain(session.bytes()) {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_lower_hex_and_stable() {
        let a = puck_id("claude", "33ef1806-76de-4243-b013-e23e9f3d14c6");
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(a, puck_id("claude", "33ef1806-76de-4243-b013-e23e9f3d14c6"));
    }

    #[test]
    fn agent_is_part_of_the_key() {
        assert_ne!(puck_id("claude", "x"), puck_id("codex", "x"));
        assert_ne!(puck_id("ab", "c"), puck_id("a", "bc"));
    }
}
