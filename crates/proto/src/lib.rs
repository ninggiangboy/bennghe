// SPDX-License-Identifier: AGPL-3.0-only
//! Generated protobuf types. The agent protocol itself is defined in phase 2.

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/bn.v1.rs"));
}

/// Protocol version this build speaks.
pub const PROTOCOL_VERSION: v1::ProtocolVersion = v1::ProtocolVersion { major: 1, minor: 0 };

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn protocol_version_round_trips() {
        let bytes = PROTOCOL_VERSION.encode_to_vec();
        assert_eq!(v1::ProtocolVersion::decode(bytes.as_slice()).unwrap(), PROTOCOL_VERSION);
    }
}
