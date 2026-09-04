//! Portable Sunshine/GameStream control plane.
//!
//! This crate owns HTTP transport, client identity, pairing, and application
//! discovery. It intentionally has no Scarlet UI, decoder, audio, or input
//! dependencies.

mod client;
mod crypto;
mod hosts;
mod session;
mod xml;

use std::net::IpAddr;

pub use client::{
    ConnectProgress, ConnectedHost, ControlError, Endpoint, GameStreamClient, ServerInfo,
};
pub use hosts::SavedHosts;
pub use session::{LaunchConfig, StreamSession};

/// A host that can provide a GameStream-compatible session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// User-visible host name.
    pub name: String,
    /// Resolved address used for control and streaming connections.
    pub address: IpAddr,
    /// Whether this client has paired with the host.
    pub paired: bool,
}

/// An application advertised by a paired streaming host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// Host-assigned application identifier.
    pub id: u32,
    /// User-visible application title.
    pub title: String,
}

/// High-level client session phase shared with the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SessionPhase {
    /// No host operation is active.
    #[default]
    Idle,
    /// Fetching host information.
    Connecting,
    /// Pairing credentials are being exchanged.
    Pairing,
    /// A paired host and its application list are available.
    Ready,
    /// A launch or resume request is in progress.
    Launching,
    /// Sunshine returned a stream URL, but the transport is not connected yet.
    SessionPrepared,
    /// The transport core owns an active stream session.
    Streaming,
    /// The previous operation failed.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::{Host, SessionPhase};
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn host_identity_is_platform_independent() {
        let host = Host {
            name: String::from("Sunshine"),
            address: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            paired: true,
        };

        assert_eq!(host.name, "Sunshine");
        assert!(host.paired);
    }

    #[test]
    fn session_starts_idle() {
        assert_eq!(SessionPhase::default(), SessionPhase::Idle);
    }
}
