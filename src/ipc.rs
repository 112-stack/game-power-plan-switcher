// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Authenticated by a per-user Windows pipe DACL; remote clients are rejected.
//! Protocol Buffers messages have a bounded length prefix, never executable text.
use prost::Message;
use std::io::{Read, Write};
pub const MAX_FRAME: usize = 16 * 1024 * 1024;
pub fn write<M: Message>(file: &mut impl Write, msg: &M) -> Result<(), String> {
    let data = msg.encode_to_vec();
    if data.len() > MAX_FRAME {
        return Err("IPC message too large".into());
    }
    file.write_all(&(data.len() as u32).to_le_bytes())
        .and_then(|_| file.write_all(&data))
        .map_err(|e| e.to_string())
}
pub fn read<M: Message + Default>(file: &mut impl Read) -> Result<M, String> {
    let mut size = [0; 4];
    file.read_exact(&mut size).map_err(|e| e.to_string())?;
    let n = u32::from_le_bytes(size) as usize;
    if n > MAX_FRAME {
        return Err("IPC frame exceeds limit".into());
    }
    let mut data = vec![0; n];
    file.read_exact(&mut data).map_err(|e| e.to_string())?;
    M::decode(data.as_slice()).map_err(|e| e.to_string())
}

/// The engine sends an initial read-only snapshot on connect. Validate both its
/// mode and any shared-memory payload before HELLO, because HELLO can start live
/// monitoring. No command bytes are sent when validation fails.
pub fn begin_session(
    file: &mut (impl Read + Write),
    dry: bool,
    paused: bool,
    mut validate_telemetry: impl FnMut(&mut crate::model::Snapshot) -> Result<(), String>,
) -> Result<crate::model::Snapshot, String> {
    let mut snapshot = read::<crate::model::Snapshot>(file)?;
    crate::runtime_scope::verify_peer(dry, snapshot.dry_run)?;
    validate_telemetry(&mut snapshot)?;
    write(
        file,
        &crate::model::Command {
            kind: crate::model::HELLO,
            flag: paused,
            text: std::process::id().to_string(),
            ..Default::default()
        },
    )?;
    Ok(snapshot)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CONFIGURE, Command, defaults};
    #[test]
    fn framed_roundtrip() {
        let c = Command {
            kind: CONFIGURE,
            config: Some(defaults()),
            ..Default::default()
        };
        let mut buf = vec![];
        write(&mut buf, &c).unwrap();
        assert_eq!(read::<Command>(&mut std::io::Cursor::new(buf)).unwrap(), c);
    }
    #[test]
    fn rejects_oversize_before_allocating() {
        assert!(
            read::<Command>(&mut std::io::Cursor::new(u32::MAX.to_le_bytes()))
                .unwrap_err()
                .contains("limit")
        );
    }
    #[test]
    fn truncated_frame_fails() {
        assert!(read::<Command>(&mut std::io::Cursor::new([10, 0, 0, 0, 8])).is_err());
    }
    #[test]
    fn malformed_protobuf_fails() {
        assert!(read::<Command>(&mut std::io::Cursor::new([1, 0, 0, 0, 255])).is_err());
    }

    struct DuplexFixture {
        incoming: std::io::Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }
    impl Read for DuplexFixture {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.incoming.read(buffer)
        }
    }
    impl Write for DuplexFixture {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.outgoing.write(buffer)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn peer(dry: bool) -> DuplexFixture {
        let mut bytes = Vec::new();
        write(
            &mut bytes,
            &crate::model::Snapshot {
                dry_run: dry,
                ..Default::default()
            },
        )
        .unwrap();
        DuplexFixture {
            incoming: std::io::Cursor::new(bytes),
            outgoing: Vec::new(),
        }
    }
    #[test]
    fn mismatched_engine_or_ring_never_receives_even_hello() {
        let mut live_peer = peer(false);
        assert!(begin_session(&mut live_peer, true, false, |_| Ok(())).is_err());
        assert!(live_peer.outgoing.is_empty());
        let mut preview_peer = peer(true);
        assert!(begin_session(&mut preview_peer, false, false, |_| Ok(())).is_err());
        assert!(preview_peer.outgoing.is_empty());
        let mut wrong_ring_peer = peer(true);
        assert!(
            begin_session(&mut wrong_ring_peer, true, false, |_| {
                crate::runtime_scope::verify_peer(true, false)
            })
            .is_err()
        );
        assert!(wrong_ring_peer.outgoing.is_empty());
    }
    #[test]
    fn matched_engine_is_checked_before_hello_can_start_monitoring() {
        let mut connection = peer(true);
        let mut checked = false;
        let snapshot = begin_session(&mut connection, true, true, |_| {
            checked = true;
            Ok(())
        })
        .unwrap();
        assert!(checked && snapshot.dry_run);
        let hello = read::<Command>(&mut std::io::Cursor::new(connection.outgoing)).unwrap();
        assert_eq!(hello.kind, crate::model::HELLO);
        assert!(hello.flag);
    }
}
