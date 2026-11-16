use std::fmt;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::auth::{proof, verify};
use crate::error::{Result, SyncError};
use crate::protocol::{FRAME_MAX, HELLO_MAX, Message, PROTOCOL_VERSION, read_frame, write_frame};
use crate::state::{DeviceId, Secret};

/// Which end of the connection this side is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Opened the connection: sends its `Hello` first.
    Dialer,
    /// Accepted the connection: answers only after verifying the dialler's `Hello`.
    Acceptor,
}

/// What a session needs to know before it starts.
#[derive(Clone)]
pub struct SessionParams {
    /// Which end of the connection this is.
    pub role: Role,
    /// The group secret.
    pub secret: Secret,
    /// Device id of this side.
    pub own_id: DeviceId,
    /// Device id of the peer, as authenticated by the transport.
    pub peer_id: DeviceId,
}

impl fmt::Debug for SessionParams {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("SessionParams")
            .field("role", &self.role)
            .field("secret", &"<redacted>")
            .field("own_id", &self.own_id)
            .field("peer_id", &self.peer_id)
            .finish()
    }
}

/// What a session reports to its host.
#[derive(Debug)]
pub enum SessionEvent {
    /// The handshake succeeded. Messages sent through `outbound` go to the peer; dropping every clone ends the session.
    Opened {
        /// The authenticated peer.
        peer: DeviceId,
        /// Channel for messages to the peer.
        outbound: UnboundedSender<Message>,
    },
    /// A message arrived from the peer.
    Received(Message),
    /// The session is over. Reported exactly once after `Opened`, never without it.
    Closed,
}

/// Authenticates, then relays messages until either side closes. `on_event` sees `Opened` once after a successful
/// handshake, then every received message, then `Closed` exactly once if `Opened` was seen.
///
/// # Errors
/// `SyncError::AuthFailed` when the peer's proof is wrong, `SyncError::Protocol` for anything else the peer
/// or the stream gets wrong. `Ok(())` when the stream ended cleanly or the outbound channel was closed.
pub async fn run_session<R, W>(
    mut reader: R,
    mut writer: W,
    params: SessionParams,
    mut on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let outcome = open_and_relay(&mut reader, &mut writer, &params, &mut on_event).await;
    let _ = writer.shutdown().await;
    outcome
}

async fn open_and_relay<R, W, E>(
    reader: &mut R,
    writer: &mut W,
    params: &SessionParams,
    on_event: &mut E,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    E: FnMut(SessionEvent),
{
    handshake(reader, writer, params).await?;
    let (outbound, outbox) = unbounded_channel();
    on_event(SessionEvent::Opened {
        peer: params.peer_id,
        outbound,
    });
    // Neither loop is cancel-safe, which is fine: when one ends, the whole session ends.
    let outcome = tokio::select! {
        outcome = receive_loop(reader, on_event) => outcome,
        outcome = send_loop(writer, outbox) => outcome,
    };
    on_event(SessionEvent::Closed);
    outcome
}

async fn receive_loop<R, E>(reader: &mut R, on_event: &mut E) -> Result<()>
where
    R: AsyncRead + Unpin,
    E: FnMut(SessionEvent),
{
    while let Some(frame) = read_frame(reader, FRAME_MAX).await? {
        match Message::decode(&frame)? {
            Message::Hello { .. } => {
                return Err(SyncError::Protocol("Hello after the handshake".to_string()));
            }
            message => on_event(SessionEvent::Received(message)),
        }
    }
    Ok(())
}

async fn send_loop<W: AsyncWrite + Unpin>(
    writer: &mut W,
    mut outbox: UnboundedReceiver<Message>,
) -> Result<()> {
    while let Some(message) = outbox.recv().await {
        write_frame(writer, &message.encode()).await?;
    }
    Ok(())
}

async fn handshake<R, W>(reader: &mut R, writer: &mut W, params: &SessionParams) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    match params.role {
        Role::Dialer => {
            send_hello(writer, params).await?;
            receive_hello(reader, params).await
        }
        Role::Acceptor => {
            receive_hello(reader, params).await?;
            send_hello(writer, params).await
        }
    }
}

async fn send_hello<W: AsyncWrite + Unpin>(writer: &mut W, params: &SessionParams) -> Result<()> {
    let hello = Message::Hello {
        version: PROTOCOL_VERSION,
        proof: proof(&params.secret, &params.own_id, &params.peer_id),
    };
    write_frame(writer, &hello.encode()).await
}

async fn receive_hello<R: AsyncRead + Unpin>(reader: &mut R, params: &SessionParams) -> Result<()> {
    let frame = read_frame(reader, HELLO_MAX)
        .await?
        .ok_or_else(|| SyncError::Protocol("stream closed before the Hello".to_string()))?;
    let Message::Hello { version, proof } = Message::decode(&frame)? else {
        return Err(SyncError::Protocol(
            "the first frame is not a Hello".to_string(),
        ));
    };
    if !verify(&params.secret, &params.peer_id, &params.own_id, &proof) {
        return Err(SyncError::AuthFailed);
    }
    if version != PROTOCOL_VERSION {
        return Err(SyncError::Protocol(format!(
            "unsupported protocol version {version}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{proof, verify};
    use crate::error::SyncError;
    use crate::protocol::{HELLO_MAX, PROTOCOL_VERSION, read_frame, write_frame};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, DuplexStream, duplex};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    const A: DeviceId = [1; 32];
    const B: DeviceId = [2; 32];
    const SECRET: Secret = [7; 32];
    const OTHER_SECRET: Secret = [8; 32];
    const PIPE_BYTES: usize = 1024;
    const WAIT: Duration = Duration::from_secs(5);

    struct Side {
        events: UnboundedReceiver<SessionEvent>,
        task: JoinHandle<Result<()>>,
    }

    fn spawn_side<R, W>(role: Role, secret: Secret, ids: (DeviceId, DeviceId), io: (R, W)) -> Side
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, events) = unbounded_channel();
        let params = SessionParams {
            role,
            secret,
            own_id: ids.0,
            peer_id: ids.1,
        };
        let task = tokio::spawn(run_session(io.0, io.1, params, move |event| {
            let _ = tx.send(event);
        }));
        Side { events, task }
    }

    fn pair(secret_a: Secret, secret_b: Secret) -> (Side, Side) {
        let (a_writer, b_reader) = duplex(PIPE_BYTES);
        let (b_writer, a_reader) = duplex(PIPE_BYTES);
        (
            spawn_side(Role::Dialer, secret_a, (A, B), (a_reader, a_writer)),
            spawn_side(Role::Acceptor, secret_b, (B, A), (b_reader, b_writer)),
        )
    }

    /// An acceptor driven by a hand-written dialler: the returned streams are the dialler's ends.
    fn acceptor_with_hand_peer() -> (DuplexStream, DuplexStream, Side) {
        let (to_acceptor, acceptor_reader) = duplex(PIPE_BYTES);
        let (acceptor_writer, from_acceptor) = duplex(PIPE_BYTES);
        let io = (acceptor_reader, acceptor_writer);
        let side = spawn_side(Role::Acceptor, SECRET, (B, A), io);
        (to_acceptor, from_acceptor, side)
    }

    async fn next_event(side: &mut Side) -> SessionEvent {
        let event = timeout(WAIT, side.events.recv()).await;
        event
            .expect("an event in time")
            .expect("the session is still reporting")
    }

    async fn opened(side: &mut Side) -> (DeviceId, UnboundedSender<Message>) {
        match next_event(side).await {
            SessionEvent::Opened { peer, outbound } => (peer, outbound),
            other => panic!("expected Opened, got {other:?}"),
        }
    }

    async fn received(side: &mut Side) -> Message {
        match next_event(side).await {
            SessionEvent::Received(message) => message,
            other => panic!("expected Received, got {other:?}"),
        }
    }

    /// Waits for the session to end and returns its result and every event it had not yet reported.
    async fn finish(mut side: Side) -> (Result<()>, Vec<SessionEvent>) {
        let result = timeout(WAIT, side.task).await;
        let result = result
            .expect("the session ends in time")
            .expect("the task joins");
        let mut events = Vec::new();
        while let Ok(event) = side.events.try_recv() {
            events.push(event);
        }
        (result, events)
    }

    fn hello_frame(version: u16, secret: &Secret) -> Vec<u8> {
        let proof = proof(secret, &A, &B);
        Message::Hello { version, proof }.encode()
    }

    async fn assert_nothing_written(mut from_acceptor: DuplexStream) {
        let mut written = Vec::new();
        let read = timeout(WAIT, from_acceptor.read_to_end(&mut written)).await;
        read.expect("the stream ends").unwrap();
        assert!(written.is_empty(), "the acceptor wrote {written:?}");
    }

    #[tokio::test]
    async fn should_open_both_sides_when_secrets_match() {
        let (mut a, mut b) = pair(SECRET, SECRET);
        assert_eq!(opened(&mut a).await.0, B);
        assert_eq!(opened(&mut b).await.0, A);
    }

    #[tokio::test]
    async fn should_carry_messages_both_ways_in_order() {
        let (mut a, mut b) = pair(SECRET, SECRET);
        let (_, a_out) = opened(&mut a).await;
        let (_, b_out) = opened(&mut b).await;
        let from_a = [
            Message::Version(b"a1".to_vec()),
            Message::Updates {
                version: vec![1],
                updates: vec![2, 3],
            },
        ];
        let from_b = Message::Peers(vec![[5; 32]]);
        for message in &from_a {
            a_out.send(message.clone()).unwrap();
        }
        b_out.send(from_b.clone()).unwrap();
        for message in &from_a {
            assert_eq!(&received(&mut b).await, message);
        }
        assert_eq!(received(&mut a).await, from_b);
    }

    #[tokio::test]
    async fn should_reject_a_wrong_secret() {
        let (a, b) = pair(SECRET, OTHER_SECRET);
        let (b_result, b_events) = finish(b).await;
        let (a_result, a_events) = finish(a).await;
        assert!(
            matches!(b_result, Err(SyncError::AuthFailed)),
            "{b_result:?}"
        );
        assert!(
            matches!(
                a_result,
                Err(SyncError::AuthFailed | SyncError::Protocol(_))
            ),
            "{a_result:?}"
        );
        assert!(a_events.is_empty() && b_events.is_empty());

        let (mut to_acceptor, from_acceptor, acceptor) = acceptor_with_hand_peer();
        write_frame(
            &mut to_acceptor,
            &hello_frame(PROTOCOL_VERSION, &OTHER_SECRET),
        )
        .await
        .unwrap();
        let (result, events) = finish(acceptor).await;
        assert!(matches!(result, Err(SyncError::AuthFailed)), "{result:?}");
        assert!(events.is_empty());
        assert_nothing_written(from_acceptor).await;
    }

    #[tokio::test]
    async fn should_reject_another_protocol_version() {
        let (mut to_acceptor, from_acceptor, acceptor) = acceptor_with_hand_peer();
        write_frame(&mut to_acceptor, &hello_frame(2, &SECRET))
            .await
            .unwrap();
        let (result, events) = finish(acceptor).await;
        assert!(matches!(result, Err(SyncError::Protocol(_))), "{result:?}");
        assert!(events.is_empty());
        assert_nothing_written(from_acceptor).await;
    }

    #[tokio::test]
    async fn should_close_when_first_frame_is_not_a_small_hello() {
        let (mut to_acceptor, from_acceptor, acceptor) = acceptor_with_hand_peer();
        write_frame(&mut to_acceptor, &[0u8; 100]).await.unwrap();
        let (result, events) = finish(acceptor).await;
        assert!(matches!(result, Err(SyncError::Protocol(_))), "{result:?}");
        assert!(events.is_empty());
        assert_nothing_written(from_acceptor).await;
    }

    #[tokio::test]
    async fn should_close_when_first_frame_is_not_a_hello() {
        let (mut to_acceptor, from_acceptor, acceptor) = acceptor_with_hand_peer();
        write_frame(&mut to_acceptor, &Message::Version(vec![1]).encode())
            .await
            .unwrap();
        let (result, events) = finish(acceptor).await;
        assert!(matches!(result, Err(SyncError::Protocol(_))), "{result:?}");
        assert!(events.is_empty());
        assert_nothing_written(from_acceptor).await;
    }

    #[tokio::test]
    async fn should_end_when_a_hello_arrives_after_the_handshake() {
        let (mut to_acceptor, mut from_acceptor, acceptor) = acceptor_with_hand_peer();
        write_frame(&mut to_acceptor, &hello_frame(PROTOCOL_VERSION, &SECRET))
            .await
            .unwrap();
        let answer = read_frame(&mut from_acceptor, HELLO_MAX)
            .await
            .unwrap()
            .unwrap();
        let Message::Hello { version, proof } = Message::decode(&answer).unwrap() else {
            panic!("the acceptor must answer with a Hello");
        };
        assert_eq!(version, PROTOCOL_VERSION);
        assert!(verify(&SECRET, &B, &A, &proof));
        write_frame(&mut to_acceptor, &hello_frame(PROTOCOL_VERSION, &SECRET))
            .await
            .unwrap();
        let (result, events) = finish(acceptor).await;
        assert!(matches!(result, Err(SyncError::Protocol(_))), "{result:?}");
        let shape = matches!(
            events.as_slice(),
            [SessionEvent::Opened { .. }, SessionEvent::Closed]
        );
        assert!(shape, "{events:?}");
    }

    #[tokio::test]
    async fn should_report_closed_once_when_the_peer_goes_away() {
        let (mut a, mut b) = pair(SECRET, SECRET);
        let (_, _a_out) = opened(&mut a).await;
        opened(&mut b).await;
        b.task.abort();
        let (result, events) = finish(a).await;
        assert!(result.is_ok(), "{result:?}");
        assert!(
            matches!(events.as_slice(), [SessionEvent::Closed]),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn should_end_when_outbound_sender_is_dropped() {
        let (mut a, mut b) = pair(SECRET, SECRET);
        let (_, a_out) = opened(&mut a).await;
        let (_, _b_out) = opened(&mut b).await;
        drop(a_out);
        let (a_result, a_events) = finish(a).await;
        assert!(a_result.is_ok(), "{a_result:?}");
        assert!(
            matches!(a_events.as_slice(), [SessionEvent::Closed]),
            "{a_events:?}"
        );
        let (b_result, b_events) = finish(b).await;
        assert!(b_result.is_ok(), "{b_result:?}");
        assert!(
            matches!(b_events.as_slice(), [SessionEvent::Closed]),
            "{b_events:?}"
        );
    }

    #[tokio::test]
    async fn should_end_when_a_frame_is_not_a_message() {
        let (mut to_acceptor, mut from_acceptor, acceptor) = acceptor_with_hand_peer();
        write_frame(&mut to_acceptor, &hello_frame(PROTOCOL_VERSION, &SECRET))
            .await
            .unwrap();
        read_frame(&mut from_acceptor, HELLO_MAX).await.unwrap();
        write_frame(&mut to_acceptor, &[9, 9]).await.unwrap();
        let (result, events) = finish(acceptor).await;
        assert!(matches!(result, Err(SyncError::Protocol(_))), "{result:?}");
        let shape = matches!(
            events.as_slice(),
            [SessionEvent::Opened { .. }, SessionEvent::Closed]
        );
        assert!(shape, "{events:?}");
    }
}
