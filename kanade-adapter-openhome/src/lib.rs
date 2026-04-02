//! kanade-adapter-openhome — OpenHome / UPnP input adapter.
//!
//! Implements a subset of the OpenHome `Transport` and `Playlist` services
//! so that JPLAY and other OpenHome control points can drive Kanade via
//! SOAP/XML over HTTP.
//!
//! Architecture role:
//! - **Input adapter**: translates SOAP XML → `CoreController` calls.
//! - **Broadcaster**: implements [`EventBroadcaster`] so the HTTP server can
//!   return fresh state on the next polled request.

pub mod broadcaster;
pub mod server;
pub mod soap;

pub use broadcaster::OpenHomeBroadcaster;
pub use server::OpenHomeServer;
