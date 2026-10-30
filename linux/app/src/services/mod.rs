//! The process-wide services behind fc-core's and fc-logic's seams (shell-design §2.4). All S0's.
//!
//! | Module | What |
//! | --- | --- |
//! | [`paths`] | the data folder and the XDG folders (§2.3) |
//! | [`settings_store`] | `settings.conf` (`glib::KeyFile`), the frozen keys, the session's two memories |
//! | [`catalogue`] | the nine languages: the desktop's language, the generated tables (§23) |
//! | [`log`] | GLib logging under `family-connect`; what a line may carry (§26) |
//! | [`files`] | `FileBlobStore` (fc-logic `BlobStore`), private folders, wiping |
//! | [`soup_http`] | `SoupHttp` — fc-core `rt::Http` over libsoup |
//! | [`soup_socket`] | `SoupSockets` / `SoupWebSocket` — fc-core `rt::WebSocketConnector` / `rt::WebSocket` |
//! | [`glib_clock`] | `GlibClock` (fc-core `rt::Clock`), `GlibZone` (fc-logic `TimeZone`), `spawn()` |

pub mod catalogue;
pub mod files;
pub mod glib_clock;
pub mod log;
pub mod paths;
pub mod settings_store;
pub mod soup_http;
pub mod soup_socket;
