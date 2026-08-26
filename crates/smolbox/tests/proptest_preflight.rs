//! Property tests for parsing smolvm version output.

use proptest::prelude::*;
use smolbox::preflight::parse_version_output;

proptest! {
    #[test]
    fn parse_version_output_never_panics(s in "\\PC{0,200}") {
        let _ = parse_version_output(&s);
    }
}
