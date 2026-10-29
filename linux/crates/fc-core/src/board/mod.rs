//! The board's rules: a port of `win/src/FamilyConnect.Core/Board` (`fc_text::board` where it has them).
//!
//! The C# was written to match `fc_text::board` — the Rust the web client runs, which is also the
//! oracle its `board-vectors.json` fixture was generated from — so nearly every rule here is CALLED
//! from there rather than written twice. What this module adds is only what `fc-text` lacks. Where
//! each C# member went:
//!
//! | C# (`FamilyConnect.Core.Board`) | Rust |
//! | --- | --- |
//! | `BoardMarks`, `BoardMarks.Zero`, `BoardMarks.Later` | `fc_text::board::Marks`, `Marks::default()`, `fc_text::board::later` |
//! | `BoardBadge.IsUnread` / `AfterShowing` | `fc_text::board::is_unread` / `marks_after_showing` |
//! | `BoardBadge.Count` | [`board_badge::count`] |
//! | `BoardPicture.Fitted` | [`board_picture::fitted`] (over `fc_text::board::fitted_picture`) |
//! | `BoardPicture.Stretch` (`"Uniform"`, XAML's) | [`board_picture::CONTENT_FIT`] (`"contain"`, GTK's and CSS's) |
//! | `BoardTasks.OnWall` / `Drawn` | `fc_text::board::WALL_TASK_LINES` / `wall_task_lines` |
//! | `BoardWall.Screens` / `Height` | `fc_text::board::WALL_SCREENS` / `wall_height` |
//! | `BoardWall.Size` | [`board_wall::size`] |
//! | `BoardWall.CompactBelow` / `IsCompact` | `fc_text::board::COMPACT_BELOW` / `is_compact` |
//! | `BoardWall.Card(size, compact)` / `TypePx(size)` | `fc_text::board::Size::frame(compact)` / `Size::type_px` |
//! | `BoardWall.Clamp` / `Origin` / `Dragged` / `FractionOf` | `fc_text::board::clamp_corner` / `origin` / `dragged` / `fraction_of` |
//! | `BoardWall.TiltDegrees` | `fc_text::board::tilt_degrees` |
//! | `NoteFitting.MinTextScale` / `FitSteps` | `fc_text::board::MIN_TEXT_SCALE` / `FIT_STEPS` |
//! | `NoteFitting.FittedScale` / `LinesThatFit` | `fc_text::board::fitted_scale` / `lines_that_fit` |
//! | `NoteSize`, `Notes.Sizes`, `SizeFrom`, `NameOf(NoteSize)`, `PatchSize` | `fc_text::board::Size`, `Size::ALL`, `Size::from_name`, `Size::name`, `Size::patch_name` |
//! | `NoteKind`, `Notes.KindFrom`, `NameOf(NoteKind)` | `fc_text::board::Kind`, `Kind::from_name`, `Kind::name` |
//! | `Notes.Colors` / `ColorHex` | `fc_text::board::COLORS` / `color_hex` (an absent name is passed as `""`) |
//! | `NoteFont`, `Notes.Fonts`, `FontFrom`, `NameOf(NoteFont)`, `PatchFont` | `fc_text::board::Font`, `Font::ALL`, `Font::from_name`, `Font::name`, `Font::patch_name` |
//! | `Notes.FontFamily` (Windows faces) | [`note_look::font_family`] (Ubuntu's) |
//! | `RsvpAnswer`, `Notes.Answers`, `AnswerFrom`, `NameOf(RsvpAnswer)` | `fc_text::board::Answer`, `Answer::ALL`, `Answer::from_name`, `Answer::name` |
//! | `Notes.Title(answer, strings)` / `GoingLine(going, maybe, strings)` | `fc_text::board::Answer::title` / `going_line` |
//! | `NoteText.MaxTextChars` / `MaxPlaceChars` / `MaxTaskItemChars` / `MaxTaskItems` / `CounterFrom` | `fc_text::board::MAX_TEXT_CHARS` / `MAX_PLACE_CHARS` / `MAX_TASK_ITEM_CHARS` / `MAX_TASK_ITEMS` / `COUNTER_FROM` |
//! | `NoteText.Scalars(text)` | `text.chars().count()` — Rust's own scalar count, which the C# imitated |
//! | `NoteText.Capped` / `Remaining` / `ShowsCounter` / `CapAtCaret` | `fc_text::board::capped` / `remaining` / `shows_counter` / `cap_at_caret` |
//! | `NoteText.DoneOf` | [`note_text::done_of`] |
//!
//! `BoardTasks.cs` and `NoteFitting.cs` have no file here: `fc_text::board` has all of both.
//!
//! The words (`Title`, `GoingLine`, `DoneOf`) take no `IStringCatalog`: they are said in the
//! reader's language through `fc_text::i18n`, which the app sets once with `use_lang` on the one
//! thread everything runs on — the same catalogue `fc_text::board`'s own sentences come from.

pub mod board_badge;
pub mod board_picture;
pub mod board_wall;
pub mod note_look;
pub mod note_text;
