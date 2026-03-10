use tracing::info;

use bunny_crypto::identity::NodeRole;
use bunny_crypto::transport::{HandshakeAccept, HandshakeInit, NodeTransport};
use bunny_crypto::types::SessionId;

use crate::error::{NetworkError, Result};
use crate::framing::{self, Frame, FrameType};

/// Perform the initiator side of a handshake over QUIC.
///
/// Opens a bi-directional stream, sends HandshakeInit, reads HandshakeAccept,
/// completes the crypto handshake, and returns the established session ID.
pub async fn initiate_handshake(
    connection: &quinn::Connection,
    transport: &NodeTransport,
    max_frame_size: u32,
) -> Result<SessionId> {
    // Generate handshake init
    let (init, ephemeral) = transport.initiate_handshake();

    // Serialize init
    let init_bytes = serde_json::to_vec(&init)
        .map_err(|e| NetworkError::HandshakeFailed(format!("serialize init: {e}")))?;

    // Open bi-directional stream for handshake
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| NetworkError::HandshakeFailed(format!("open stream: {e}")))?;

    // Send HandshakeInit frame
    let frame = Frame::new(FrameType::HandshakeInit, init_bytes);
    framing::write_frame(&mut send, &frame).await?;
    send.finish()
        .map_err(|e| NetworkError::HandshakeFailed(format!("finish send: {e}")))?;

    // Read HandshakeAccept frame
    let accept_frame = framing::read_frame(&mut recv, max_frame_size).await?;
    if accept_frame.frame_type != FrameType::HandshakeAccept {
        return Err(NetworkError::HandshakeFailed(format!(
            "expected HandshakeAccept, got {:?}",
            accept_frame.frame_type
        )));
    }

    // Deserialize accept
    let accept: HandshakeAccept = serde_json::from_slice(&accept_frame.payload)
        .map_err(|e| NetworkError::HandshakeFailed(format!("deserialize accept: {e}")))?;

    // Complete crypto handshake
    let session_id = transport
        .complete_handshake(&accept, ephemeral)
        .await
        .map_err(|e| NetworkError::HandshakeFailed(format!("complete handshake: {e}")))?;

    info!(session = ?session_id, "QUIC handshake completed (initiator)");
    Ok(session_id)
}

/// Perform the responder side of a handshake over QUIC.
///
/// Reads HandshakeInit from a bi-directional stream, performs the crypto handshake,
/// sends HandshakeAccept back, and returns the established session ID along with
/// the peer's authenticated role.
pub async fn accept_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    transport: &NodeTransport,
    max_frame_size: u32,
) -> Result<(SessionId, NodeRole)> {
    // Read HandshakeInit frame
    let init_frame = framing::read_frame(recv, max_frame_size).await?;
    if init_frame.frame_type != FrameType::HandshakeInit {
        return Err(NetworkError::HandshakeFailed(format!(
            "expected HandshakeInit, got {:?}",
            init_frame.frame_type
        )));
    }

    // Deserialize init
    let init: HandshakeInit = serde_json::from_slice(&init_frame.payload)
        .map_err(|e| NetworkError::HandshakeFailed(format!("deserialize init: {e}")))?;

    // Capture peer's authenticated role before the handshake consumes init
    let peer_role = init.announcement.identity.role;

    // Accept crypto handshake
    let accept = transport
        .accept_handshake(&init)
        .await
        .map_err(|e| NetworkError::HandshakeFailed(format!("accept handshake: {e}")))?;

    let session_id = accept.session_id.clone();

    // Serialize and send accept
    let accept_bytes = serde_json::to_vec(&accept)
        .map_err(|e| NetworkError::HandshakeFailed(format!("serialize accept: {e}")))?;
    let frame = Frame::new(FrameType::HandshakeAccept, accept_bytes);
    framing::write_frame(send, &frame).await?;
    send.finish()
        .map_err(|e| NetworkError::HandshakeFailed(format!("finish send: {e}")))?;

    info!(session = ?session_id, peer_role = ?peer_role, "QUIC handshake completed (responder)");
    Ok((session_id, peer_role))
}

/// Send an encrypted envelope over a QUIC uni-directional stream.
pub async fn send_envelope(
    connection: &quinn::Connection,
    transport: &NodeTransport,
    session_id: &SessionId,
    payload: &[u8],
) -> Result<()> {
    let envelope = transport.send(session_id, payload).await?;
    let envelope_bytes = envelope.to_bytes();

    let mut send = connection
        .open_uni()
        .await
        .map_err(|e| NetworkError::Connection(format!("open uni stream: {e}")))?;

    let frame = Frame::new(FrameType::Envelope, envelope_bytes);
    framing::write_frame(&mut send, &frame).await?;
    send.finish()
        .map_err(|e| NetworkError::Connection(format!("finish send: {e}")))?;

    Ok(())
}

/// Send a cloaked envelope over a QUIC uni-directional stream.
pub async fn send_cloaked_envelope(
    connection: &quinn::Connection,
    transport: &NodeTransport,
    session_id: &SessionId,
    route_tag: bunny_crypto::OpaqueRouteTag,
    payload: &[u8],
) -> Result<()> {
    let envelope = transport
        .send_cloaked(session_id, route_tag, payload)
        .await?;
    let envelope_bytes = envelope.to_bytes();

    let mut send = connection
        .open_uni()
        .await
        .map_err(|e| NetworkError::Connection(format!("open uni stream: {e}")))?;

    let frame = Frame::new(FrameType::CloakedEnvelope, envelope_bytes);
    framing::write_frame(&mut send, &frame).await?;
    send.finish()
        .map_err(|e| NetworkError::Connection(format!("finish send: {e}")))?;

    Ok(())
}
