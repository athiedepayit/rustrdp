//! Clipboard passthrough for RDP sessions.
//!
//! This module implements [`CliprdrBackend`] using the `arboard` crate for
//! host-OS clipboard access.  Only plain-text (Unicode) is supported for now.
//!
//! # Design
//!
//! The backend runs inside the worker thread but needs to call back into the
//! [`CliprdrClient`] (which lives inside the `ActiveStage`).  Because Rust
//! doesn't allow simultaneous mutable borrows we use a small channel:
//!
//! ```text
//!  CliprdrBackend callbacks
//!        │
//!        │  ClipboardEvent via mpsc::Sender
//!        ▼
//!  session loop  ─── active_stage.get_svc_processor_mut::<CliprdrClient>()
//!                          │
//!                          └─ initiate_copy / submit_format_data / etc.
//!                          └─ process_svc_processor_messages  → wire
//! ```

use std::sync::mpsc::Sender;

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, OwnedFormatDataResponse,
};
use ironrdp_cliprdr::pdu::{ClipboardGeneralCapabilityFlags, LockDataId};
use ironrdp_core::impl_as_any;

/// Events produced by the [`AppClipboardBackend`] and consumed by the session
/// worker loop to drive the [`CliprdrClient`].
#[derive(Debug)]
pub enum ClipboardEvent {
    /// Backend wants to announce available local formats (triggered by
    /// `on_ready` or `on_request_format_list`).
    InitiateCopy(Vec<ClipboardFormat>),
    /// Backend has fetched local clipboard data and wants to send it as a
    /// `FormatDataResponse` (triggered by `on_format_data_request`).
    SubmitFormatData(OwnedFormatDataResponse),
    /// Backend wants to request clipboard data from the server in the given
    /// format (triggered by `on_remote_copy`).
    RequestFormatData(ClipboardFormatId),
}

/// [`CliprdrBackend`] implementation that bridges the RDP clipboard channel to
/// the host OS clipboard via `arboard`.
#[derive(Debug)]
pub struct AppClipboardBackend {
    /// Channel used to hand events back to the session loop.
    tx: Sender<ClipboardEvent>,
    /// Temporary directory advertised to the server (not used for text-only).
    temp_dir: String,
    /// Negotiated capability flags (stored for reference).
    capabilities: ClipboardGeneralCapabilityFlags,
}

impl AppClipboardBackend {
    pub fn new(tx: Sender<ClipboardEvent>) -> Self {
        Self {
            tx,
            temp_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            capabilities: ClipboardGeneralCapabilityFlags::empty(),
        }
    }
}

impl_as_any!(AppClipboardBackend);

impl CliprdrBackend for AppClipboardBackend {
    fn temporary_directory(&self) -> &str {
        &self.temp_dir
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // Advertise long format names; skip file-copy extensions for simplicity.
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }

    fn on_ready(&mut self) {
        // Announce that we have Unicode text available as soon as the clipboard
        // channel is ready.  This allows the server to request our clipboard
        // content (the host → remote paste direction).
        let formats = vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
        let _ = self.tx.send(ClipboardEvent::InitiateCopy(formats));
    }

    fn on_request_format_list(&mut self) {
        // Tell the server we have Unicode text available.
        let formats = vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
        let _ = self.tx.send(ClipboardEvent::InitiateCopy(formats));
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        self.capabilities = capabilities;
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // The remote clipboard changed.  If it contains Unicode text, request
        // that data now so it ends up in the host OS clipboard (remote → host
        // copy direction).
        let has_unicode = available_formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT);
        if has_unicode {
            let _ = self.tx.send(ClipboardEvent::RequestFormatData(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        // The server wants our clipboard content.
        let response = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
                Ok(text) => OwnedFormatDataResponse::new_unicode_string(&text),
                Err(_) => OwnedFormatDataResponse::new_error(),
            }
        } else {
            OwnedFormatDataResponse::new_error()
        };
        let _ = self.tx.send(ClipboardEvent::SubmitFormatData(response));
    }

    fn on_format_data_response(&mut self, response: ironrdp_cliprdr::pdu::FormatDataResponse<'_>) {
        // We received clipboard data from the remote.  Write it to the local
        // clipboard if it looks like a Unicode string.
        if response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(text);
            }
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        // File transfer not supported — respond with an error.
        let response = FileContentsResponse::new_error(request.stream_id);
        // We need to send this back, but we don't have a SubmitFileContents
        // event path.  For now just drop it; the server will time out.
        let _ = response; // suppress unused warning
    }

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}
    fn on_unlock(&mut self, _data_id: LockDataId) {}
}
