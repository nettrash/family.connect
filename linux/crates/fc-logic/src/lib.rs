//! Family Connect for Linux — everything the window does, with no window: a port of
//! `win/src/FamilyConnect.App.Logic` (see `linux/docs/port-plan.md`, wave 2). Nothing here touches
//! GTK, GStreamer or (but for `folder_media_store`) the file system directly; the shell implements the
//! seams.
//!
//! Layer 0 — the files that depend on no other App.Logic file — plus the Linux rule sets the shell
//! design asks for (`docs/shell-design.md` Appendix B): `tray_rules`, `startup_setting` and
//! `global_hot_key_rules` in Linux terms, `notification_target`, and `date_culture` (the C#'s
//! `CultureInfo`/`TimeZoneInfo`, which Rust does not have).
//!
//! The window's foundations, over the stores and the client of `fc-core`: the wall (`board`,
//! `note_hand`, `wall_text`), the family's screens (`family`), the media pump (`media_outbox`), the
//! downloaded bytes and the faces (`attachment_cache`, `avatars`), and one open chat
//! (`conversation`, `quotes`, `typing`, `poll_voting`, `open_polls`, `threads`, `timeline`). Where
//! the C# reads or writes files it does so through an injected interface, and so does this:
//! `attachment_cache::BlobStore` and `media_outbox::MediaStore` are the shell's to implement.
//!
//! Every C# method that touches the SQLite cache answers a `StoreResult` here — `Err` is the cache
//! refusing, which the C# let out as an exception — around the C#'s own answer: `Option<ApiError>`
//! for a `Task<ApiError?>`, and a `Result` for an `(answer, error)` pair. Cancelling is dropping the
//! future, and every model takes `&self`, its own state in cells, so the shell holds it in an `Rc`.
//!
//! Animations, attachment files and a few independent rules: an animated picture taken apart
//! (`animated_pictures`), composited and rendered (`animation_frames`), timed (`animation_timing`)
//! and made into the protocol's video (`animation_preparing`, whose `StillDecoder` and
//! `AnimationEncoder` are the shell's — gdk-pixbuf and GStreamer); a tile's source and a saved
//! file's name (`attachment_files`); the staged sends on disk (`folder_media_store` — the one module
//! here that touches the file system, because the C# class IS the file-backed `MediaStore`); the
//! link cards (`link_previews`, over an `rt::Http`); the viewer's album (`media_album`); a call
//! record's marks (`call_records`); the member limit's debounced write (`cap_draft`); and which
//! drawn rows a redraw keeps (`conversation_rows`).
//!
//! The session layer, which composes the core's pieces: the account and its gate (`app_session`,
//! `join_memory`), several families (`families`), the doorway (`doorway`), the live connection
//! (`live_connection` — socket, send pipeline, router, resync and the passes, wired as
//! `linux/docs/carry-forward.md` says), the chat list (`chat_list`), groups (`groups`), the sticker
//! pack (`pack`), the assistant's consent and lookups (`assistant_consent`, `lookups`,
//! `consented_ask`, `backdrop_consent`, `picture_hint`, `greeting_weather`), the composer's helpers
//! (`composer_mentions`, `attach_menu`), a bubble's body (`bubble_body`), notifications
//! (`notifications`, `toast_arguments`), the family console (`family_console`), the board's sheet
//! (`note_draft`, `note_preview`, `photo_pinning`) and paid families — the door, the unpaid banner,
//! the allowance and the refusals (`paid_families`).
//!
//! The recording package — voice notes, video messages and the composer: the Send slot and its
//! guards (`composer_button`, over `fc_text::record`), the composer's row and column
//! (`composer_row`), what a picked file becomes (`composer_staging`), a voice note's rules, its
//! start and its end (`voice_notes`, `not_sent`), the voice messages kept when a recording was
//! interrupted (`parked_recordings` — file-backed like `folder_media_store`), the voice bubble, the
//! live meter and the sender's waveform (`voice_look`), a message's menu (`message_menu`), the
//! transcripts and the sound a device sends for one (`transcripts`, `transcript_sound`), the
//! encodings GStreamer is asked for and Rule E's sound tracks (`media_encoding`,
//! `container_sound`), and the video message: the recorder's rules and state machine
//! (`round_video_rules`, `round_recorder`), its layout (`recorder_frame`), and a circle's look and
//! in-place playback (`round_look`, `round_playback`). The platform halves are traits the shell
//! implements with GStreamer and the camera portal: `voice_notes::VoiceRecorder` (with
//! `voice_look::VoiceMeter`, `voice_look::VoiceDecoder` and `voice_notes::Inhibitor`),
//! `round_recorder::VideoMessageRecorder`, `round_playback::RoundPlayers`,
//! `transcript_sound::SoundTrackMaker` and `media_encoding::preparing::MediaPreparing` (the
//! transcode of picked media, driven by `preparing::prepare_video`/`prepare_sound`).

pub mod animated_pictures;
pub mod animation_frames;
pub mod animation_mp4;
pub mod animation_preparing;
pub mod animation_timing;
pub mod app_session;
pub mod app_version_text;
pub mod assistant_answers;
pub mod assistant_consent;
pub mod attach_menu;
pub mod attachment_cache;
pub mod attachment_files;
pub mod avatars;
pub mod backdrop_consent;
pub mod board;
pub mod bubble_body;
pub mod bubble_rules;
pub mod call_records;
pub mod calls;
pub mod cap_draft;
pub mod capital_field;
pub mod capitals;
pub mod chat_list;
pub mod composer_button;
pub mod composer_drafts;
pub mod composer_mentions;
pub mod composer_row;
pub mod composer_staging;
pub mod consented_ask;
pub mod container_sound;
pub mod conversation;
pub mod conversation_rows;
pub mod date_culture;
pub mod doorway;
pub mod drawn_copies;
pub mod emoji_typing;
pub mod event_text;
pub mod families;
pub mod family;
pub mod family_console;
pub mod faststart;
pub mod folder_media_store;
pub mod frame_turn;
pub mod global_hot_key_rules;
pub mod greeting_weather;
pub mod groups;
pub mod join_memory;
pub mod link_previews;
pub mod live_connection;
pub mod lookups;
pub mod map_view;
pub mod media_album;
pub mod media_encoding;
pub mod media_outbox;
pub mod message_menu;
pub mod new_line_keys;
pub mod not_sent;
pub mod note_draft;
pub mod note_hand;
pub mod note_preview;
pub mod notification_target;
pub mod notifications;
pub mod open_polls;
pub mod pack;
pub mod paid_families;
pub mod parked_recordings;
pub mod peer_reads;
pub mod photo_pinning;
pub mod picture_hint;
pub mod picture_memory;
pub mod poll_draft;
pub mod poll_text;
pub mod poll_voting;
pub mod quotes;
pub mod recorder_frame;
pub mod rich_drawing;
pub mod round_look;
pub mod round_playback;
pub mod round_recorder;
pub mod round_video_rules;
pub mod settings;
pub mod startup_setting;
pub mod sticker_look;
pub mod threads;
pub mod timeline;
pub mod toast_arguments;
pub mod transcode_attempts;
pub mod transcript_sound;
pub mod transcripts;
pub mod tray_rules;
pub mod typing;
pub mod voice_look;
pub mod voice_notes;
pub mod wall_text;
