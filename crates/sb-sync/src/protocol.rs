use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Result, SyncError};
use crate::state::DeviceId;

/// Version of the wire protocol spoken after the `Hello` exchange.
pub const PROTOCOL_VERSION: u16 = 1;
/// Largest first frame a peer may send, before it has proven group membership.
pub const HELLO_MAX: usize = 64;
/// Largest frame a peer may send after authentication.
pub const FRAME_MAX: usize = 1 << 30;
/// Transport protocol name that both ends announce.
pub const ALPN: &[u8] = b"secondbrain/sync/1";

/// One message of the sync protocol, carried in one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Authentication: the protocol version and the proof of group membership.
    Hello {
        /// Protocol version of the sender.
        version: u16,
        /// Proof that the sender knows the group secret.
        proof: [u8; 32],
    },
    /// The sender's document version: "send me what I lack".
    Version(Vec<u8>),
    /// Document updates, with the sender's document version.
    Updates {
        /// The sender's document version.
        version: Vec<u8>,
        /// Encoded document updates, possibly empty.
        updates: Vec<u8>,
    },
    /// Device ids the sender knows.
    Peers(Vec<DeviceId>),
}

impl Message {
    /// Encodes the message as a frame payload: a type byte, then the body.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Message::Hello { version, proof } => {
                [&[TYPE_HELLO][..], &version.to_be_bytes(), proof].concat()
            }
            Message::Version(version) => [&[TYPE_VERSION][..], version].concat(),
            Message::Updates { version, updates } => {
                // A version past 4 GiB makes the payload exceed FRAME_MAX, so `write_frame` refuses it anyway.
                let version_len = u32::try_from(version.len()).unwrap_or(u32::MAX);
                [
                    &[TYPE_UPDATES][..],
                    &version_len.to_be_bytes(),
                    version,
                    updates,
                ]
                .concat()
            }
            Message::Peers(ids) => [&[TYPE_PEERS][..], &ids.concat()].concat(),
        }
    }

    /// Decodes a frame payload.
    ///
    /// # Errors
    /// `SyncError::Protocol` for anything malformed.
    pub fn decode(payload: &[u8]) -> Result<Message> {
        let (&kind, body) = payload
            .split_first()
            .ok_or_else(|| malformed("empty message"))?;
        match kind {
            TYPE_HELLO => decode_hello(body),
            TYPE_VERSION => Ok(Message::Version(body.to_vec())),
            TYPE_UPDATES => decode_updates(body),
            TYPE_PEERS => decode_peers(body),
            other => Err(malformed(&format!("unknown message type {other}"))),
        }
    }
}

const TYPE_HELLO: u8 = 1;
const TYPE_VERSION: u8 = 2;
const TYPE_UPDATES: u8 = 3;
const TYPE_PEERS: u8 = 4;

fn malformed(reason: &str) -> SyncError {
    SyncError::Protocol(reason.to_string())
}

fn decode_hello(body: &[u8]) -> Result<Message> {
    let hello = body
        .split_first_chunk::<2>()
        .and_then(|(version, proof)| Some((version, <&[u8; 32]>::try_from(proof).ok()?)))
        .ok_or_else(|| malformed("Hello must be 35 bytes"))?;
    Ok(Message::Hello {
        version: u16::from_be_bytes(*hello.0),
        proof: *hello.1,
    })
}

fn decode_updates(body: &[u8]) -> Result<Message> {
    let (len, rest) = body
        .split_first_chunk::<4>()
        .ok_or_else(|| malformed("Updates is missing its version length"))?;
    let (version, updates) = usize::try_from(u32::from_be_bytes(*len))
        .ok()
        .and_then(|len| rest.split_at_checked(len))
        .ok_or_else(|| malformed("Updates version runs past the end"))?;
    Ok(Message::Updates {
        version: version.to_vec(),
        updates: updates.to_vec(),
    })
}

fn decode_peers(body: &[u8]) -> Result<Message> {
    let (ids, rest) = body.as_chunks::<32>();
    if !rest.is_empty() {
        return Err(malformed("Peers is not a whole number of device ids"));
    }
    Ok(Message::Peers(ids.to_vec()))
}

fn stream_error(error: &std::io::Error) -> SyncError {
    SyncError::Protocol(format!("stream error: {error}"))
}

/// Writes one frame: a 4-byte big-endian length, then the payload.
///
/// # Errors
/// `SyncError::Protocol` (nothing written) when the payload exceeds `FRAME_MAX`, or when the stream fails.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> Result<()> {
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|_| payload.len() <= FRAME_MAX)
        .ok_or_else(|| malformed("frame exceeds the size limit"))?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| stream_error(&e))?;
    writer
        .write_all(payload)
        .await
        .map_err(|e| stream_error(&e))?;
    writer.flush().await.map_err(|e| stream_error(&e))
}

/// Reads one frame of at most `max` bytes. `None` means the stream ended cleanly before a frame started.
///
/// # Errors
/// `SyncError::Protocol` when the announced length exceeds `max` (the body is not read), or when the
/// stream fails or ends inside a frame.
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Option<Vec<u8>>> {
    let mut prefix = [0u8; 4];
    let first = reader
        .read(&mut prefix[..1])
        .await
        .map_err(|e| stream_error(&e))?;
    if first == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut prefix[1..])
        .await
        .map_err(|e| stream_error(&e))?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len > max {
        return Err(malformed(&format!(
            "frame of {len} bytes exceeds the limit of {max}"
        )));
    }
    // `take` grows the buffer as bytes arrive: a peer cannot make us allocate `len` up front.
    let mut payload = Vec::new();
    (&mut *reader)
        .take(len as u64)
        .read_to_end(&mut payload)
        .await
        .map_err(|e| stream_error(&e))?;
    if payload.len() != len {
        return Err(malformed("stream ended inside a frame"));
    }
    Ok(Some(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SyncError;
    use std::time::Duration;
    use tokio::io::{AsyncWriteExt, duplex};
    use tokio::time::timeout;

    fn is_protocol_error<T: std::fmt::Debug>(result: Result<T>) -> bool {
        matches!(result, Err(SyncError::Protocol(_)))
    }

    #[test]
    fn should_round_trip_every_message() {
        let messages = [
            Message::Hello {
                version: 1,
                proof: [7; 32],
            },
            Message::Version(vec![]),
            Message::Version(vec![1, 2, 3]),
            Message::Updates {
                version: vec![],
                updates: vec![],
            },
            Message::Updates {
                version: vec![4, 5],
                updates: vec![],
            },
            Message::Updates {
                version: vec![],
                updates: vec![6, 7, 8],
            },
            Message::Updates {
                version: vec![4, 5],
                updates: vec![6, 7, 8],
            },
            Message::Peers(vec![]),
            Message::Peers(vec![[1; 32], [2; 32]]),
        ];
        for message in messages {
            assert_eq!(Message::decode(&message.encode()).unwrap(), message);
        }
    }

    #[test]
    fn should_encode_hello_exactly() {
        let hello = Message::Hello {
            version: 1,
            proof: [9; 32],
        };
        let mut expected = vec![1, 0, 1];
        expected.extend([9u8; 32]);
        assert_eq!(hello.encode(), expected);
    }

    #[test]
    fn should_reject_malformed_payloads() {
        let mut updates_past_end = vec![3, 0, 0, 0, 5];
        updates_past_end.extend([1, 2]);
        let malformed: [Vec<u8>; 7] = [
            vec![],
            vec![0],
            vec![9, 1, 2],
            [vec![1], vec![0; 33]].concat(),
            updates_past_end,
            vec![3, 0, 0],
            [vec![4], vec![0; 33]].concat(),
        ];
        for payload in malformed {
            assert!(
                is_protocol_error(Message::decode(&payload)),
                "{payload:?} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn should_frame_and_unframe() {
        let mut written = Vec::new();
        write_frame(&mut written, b"abc").await.unwrap();
        assert_eq!(written, [0, 0, 0, 3, b'a', b'b', b'c']);
        let mut reader = written.as_slice();
        let frame = read_frame(&mut reader, FRAME_MAX).await.unwrap();
        assert_eq!(frame.as_deref(), Some(&b"abc"[..]));
        assert_eq!(read_frame(&mut reader, FRAME_MAX).await.unwrap(), None);
    }

    #[tokio::test]
    async fn should_refuse_oversized_frames() {
        // The stream stays open and silent after the prefix: reading the body would hang, not fail.
        let (mut peer, mut reader) = duplex(64);
        peer.write_all(&0x4000_0001u32.to_be_bytes()).await.unwrap();
        let result = timeout(Duration::from_secs(5), read_frame(&mut reader, FRAME_MAX))
            .await
            .expect("must not wait for the body");
        assert!(is_protocol_error(result));

        let mut frame = 65u32.to_be_bytes().to_vec();
        frame.extend([0u8; 65]);
        let result = read_frame(&mut frame.as_slice(), HELLO_MAX).await;
        assert!(is_protocol_error(result));
    }

    #[tokio::test]
    async fn should_fail_on_truncated_frame() {
        let mut body_cut = &[0u8, 0, 0, 10, 1, 2, 3, 4][..];
        assert!(is_protocol_error(
            read_frame(&mut body_cut, FRAME_MAX).await
        ));
        let mut prefix_cut = &[0u8, 0][..];
        assert!(is_protocol_error(
            read_frame(&mut prefix_cut, FRAME_MAX).await
        ));
    }
}
