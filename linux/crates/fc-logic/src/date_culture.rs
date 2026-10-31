//! The reader's way of writing a date and a time, and the zone they live in — what the C# reads
//! from a `CultureInfo` and a `TimeZoneInfo` (`EventText.cs`, `BirthdayRules` in `Settings.cs`).
//!
//! NOT A C# FILE: a seam of the Linux port. Rust has no culture and `time` has no zone database,
//! and the shell must not format a date with `strftime`'s locale alone (shell-design §23,
//! "Formats"), so the slice of .NET's `DateTimeFormatInfo` the window reads is plain data here
//! ([`DateCulture`]), with the part of .NET's custom-format grammar the clients use
//! ([`DateCulture::format`]), and a zone is anything that can say its offset at an instant
//! ([`TimeZone`] — the shell answers it from `glib::TimeZone::local()`).
//!
//! THE TABLES ARE .NET'S OWN, measured: [`DateCulture::named`] knows the ten cultures the nine
//! languages are read in (en-US, en-GB, de-DE, es-ES, fr-FR, ja-JP, ru-RU, sr-Cyrl-RS,
//! sr-Latn-RS, zh-CN) and the invariant one, printed from `CultureInfo.GetCultureInfo(name)
//! .DateTimeFormat` on .NET 10.0.402 (ICU) — so a date reads on Linux exactly as Windows writes it,
//! U+202F before "PM" included. [`DateCulture::for_locale`] maps a POSIX locale name (`LC_TIME`,
//! `de_AT.UTF-8`, `sr_RS@latin`) onto the nearest of them.

use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// A zone: what the clock on the reader's wall says at an instant (`TimeZoneInfo.ConvertTime`).
pub trait TimeZone {
    /// The zone's offset from UTC at `instant` — daylight saving included.
    fn offset_at(&self, instant: OffsetDateTime) -> UtcOffset;
}

/// A fixed offset is a zone (`TimeZoneInfo.Utc`, `TimeZoneInfo.CreateCustomTimeZone(…)`).
impl TimeZone for UtcOffset {
    fn offset_at(&self, _instant: OffsetDateTime) -> UtcOffset {
        *self
    }
}

/// `instant` as the reader's wall clock shows it.
pub fn local(instant: OffsetDateTime, zone: &dyn TimeZone) -> OffsetDateTime {
    instant.to_offset(zone.offset_at(instant))
}

/// The slice of a .NET `DateTimeFormatInfo` the window reads. Month arrays are January first,
/// day arrays Sunday first, as .NET's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateCulture {
    /// The .NET name ("en-GB"); "" for the invariant culture.
    pub name: String,
    /// `ShortTimePattern` — standard format `t`.
    pub short_time_pattern: String,
    /// `ShortDatePattern` — standard format `d` (measured on .NET 10.0.13 for the chat list's
    /// older rows, `RowTimeText`).
    pub short_date_pattern: String,
    /// `MonthDayPattern` — standard format `M`.
    pub month_day_pattern: String,
    /// `LongDatePattern` — standard format `D`, the weekday included (measured on .NET 10.0.13 for
    /// the paid families' dates, [`day_month_year_pattern`](Self::day_month_year_pattern)).
    pub long_date_pattern: String,
    /// What `:` writes in a custom pattern.
    pub time_separator: String,
    /// What `/` writes in a custom pattern.
    pub date_separator: String,
    pub am_designator: String,
    pub pm_designator: String,
    pub month_names: [String; 12],
    /// The month as it reads after a day's number ("12 марта"), where a language declines it.
    pub month_genitive_names: [String; 12],
    pub abbreviated_month_names: [String; 12],
    pub abbreviated_month_genitive_names: [String; 12],
    pub day_names: [String; 7],
    pub abbreviated_day_names: [String; 7],
}

impl DateCulture {
    /// `CultureInfo.InvariantCulture`.
    pub fn invariant() -> DateCulture {
        DateCulture::from_table(&INVARIANT)
    }

    /// One of the measured cultures by its .NET name, matched without regard to case; `None` for
    /// any other.
    pub fn named(name: &str) -> Option<DateCulture> {
        TABLES
            .iter()
            .find(|table| table.name.eq_ignore_ascii_case(name))
            .map(|table| DateCulture::from_table(table))
    }

    /// The nearest measured culture for a POSIX locale name as `LC_TIME` gives it — `de_AT.UTF-8`,
    /// `sr_RS@latin`, `zh_TW`, `C`. The language decides (de, es, fr, ja, ru, zh); English is
    /// en-US in the United States (and with no region) and en-GB elsewhere; Serbian is Latin with
    /// `@latin` or a `Latn` script, Cyrillic otherwise. Anything else — `C`, `POSIX`, a language
    /// the apps do not speak — reads as en-US, the language the app then speaks.
    pub fn for_locale(locale: &str) -> DateCulture {
        let (base, modifier) = match locale.split_once('@') {
            Some((base, modifier)) => (base, modifier),
            None => (locale, ""),
        };
        let base = base.split('.').next().unwrap_or(base);
        let mut parts = base.split(['_', '-']);
        let language = parts.next().unwrap_or("").to_ascii_lowercase();
        let rest: Vec<String> = parts.map(str::to_ascii_lowercase).collect();
        let script_latin =
            modifier.eq_ignore_ascii_case("latin") || rest.iter().any(|p| p == "latn");
        let region = rest
            .iter()
            .find(|part| part.len() == 2)
            .cloned()
            .unwrap_or_default();
        let name = match language.as_str() {
            "en" if region.is_empty() || region == "us" => "en-US",
            "en" => "en-GB",
            "de" => "de-DE",
            "es" => "es-ES",
            "fr" => "fr-FR",
            "ja" => "ja-JP",
            "ru" => "ru-RU",
            "sr" if script_latin => "sr-Latn-RS",
            "sr" => "sr-Cyrl-RS",
            "zh" => "zh-CN",
            _ => "en-US",
        };
        DateCulture::named(name).unwrap_or_else(DateCulture::invariant)
    }

    fn from_table(table: &Table) -> DateCulture {
        fn own<const N: usize>(names: &[&str; N]) -> [String; N] {
            names.map(str::to_string)
        }
        DateCulture {
            name: table.name.to_string(),
            short_time_pattern: table.short_time_pattern.to_string(),
            short_date_pattern: table.short_date_pattern.to_string(),
            month_day_pattern: table.month_day_pattern.to_string(),
            long_date_pattern: table.long_date_pattern.to_string(),
            time_separator: table.time_separator.to_string(),
            date_separator: table.date_separator.to_string(),
            am_designator: table.am_designator.to_string(),
            pm_designator: table.pm_designator.to_string(),
            month_names: own(&table.month_names),
            month_genitive_names: own(&table.month_genitive_names),
            abbreviated_month_names: own(&table.abbreviated_month_names),
            abbreviated_month_genitive_names: own(&table.abbreviated_month_genitive_names),
            day_names: own(&table.day_names),
            abbreviated_day_names: own(&table.abbreviated_day_names),
        }
    }

    /// The long date without its weekday — the day, the month and the year in the culture's own
    /// order: "January 4, 2027", "4. Januar 2027", "4 января 2027 г.", "2027年1月4日". .NET has no
    /// such standard format, so the weekday is taken off the long date where every measured
    /// culture puts it — first with ", " or " " after it, or last with " " before it — and a
    /// pattern with no weekday there is kept whole.
    pub fn day_month_year_pattern(&self) -> String {
        let long = self.long_date_pattern.as_str();
        long.strip_prefix("dddd, ")
            .or_else(|| long.strip_prefix("dddd "))
            .or_else(|| long.strip_suffix(" dddd"))
            .unwrap_or(long)
            .to_string()
    }

    /// `month` (1–12) standing alone, as a picker lists it (`GetMonthName`); "" outside 1–12.
    pub fn month_name(&self, month: u8) -> &str {
        match month {
            1..=12 => &self.month_names[usize::from(month - 1)],
            _ => "",
        }
    }

    /// `at.ToString(pattern, culture)` for the part of .NET's grammar the window writes:
    ///
    /// - the standard formats `t` (the short time), `d` (the short date) and `M`/`m` (the month and
    ///   day), as a pattern of that one letter — so the day's number alone is `%d`, as in .NET;
    /// - the custom specifiers `d` `dd` (the day's number), `ddd` `dddd` (its name), `M` `MM`,
    ///   `MMM` `MMMM` (the month's — GENITIVE when the pattern also holds a `d` or `dd`, .NET's
    ///   `IsUseGenitiveForm`, so "d MMMM" is "12 марта" and "MMMM" alone "март"), `y` `yy` `yyyy`,
    ///   `h` `hh` `H` `HH` `m` `mm` `s` `ss` (at most two digits, as .NET pads), `t` `tt` (the
    ///   designator, or its first character), `:` and `/` (the culture's separators), `'…'` and
    ///   `"…"` (literal, `\` escaping inside), `\x` (a literal character), and `%x` (one specifier
    ///   alone);
    /// - any other character is written as it is.
    ///
    /// Where .NET would throw — an unterminated quote, a trailing `\` or `%` — what is there is
    /// written literally instead: a date is something to show, not a panic.
    pub fn format(&self, at: PrimitiveDateTime, pattern: &str) -> String {
        match pattern {
            "t" => return self.format(at, &self.short_time_pattern),
            "d" => return self.format(at, &self.short_date_pattern),
            "M" | "m" => return self.format(at, &self.month_day_pattern),
            _ => {}
        }
        let chars: Vec<char> = pattern.chars().collect();
        let mut out = String::new();
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            match ch {
                '\'' | '"' => {
                    index += 1;
                    while index < chars.len() && chars[index] != ch {
                        if chars[index] == '\\' && index + 1 < chars.len() {
                            index += 1;
                        }
                        out.push(chars[index]);
                        index += 1;
                    }
                    // Past the closing quote (or the end).
                    index += 1;
                }
                '\\' => {
                    if let Some(&next) = chars.get(index + 1) {
                        out.push(next);
                        index += 2;
                    } else {
                        out.push('\\');
                        index += 1;
                    }
                }
                '%' => match chars.get(index + 1) {
                    Some(&next) if next != '%' => {
                        self.token(at, &[next], 0, 1, &mut out);
                        index += 2;
                    }
                    _ => {
                        out.push('%');
                        index += 1;
                    }
                },
                _ => {
                    let length = chars[index..].iter().take_while(|&&c| c == ch).count();
                    self.token(at, &chars, index, length, &mut out);
                    index += length;
                }
            }
        }
        out
    }

    /// One run of `length` copies of `chars[index]`.
    fn token(
        &self,
        at: PrimitiveDateTime,
        chars: &[char],
        index: usize,
        length: usize,
        out: &mut String,
    ) {
        let ch = chars[index];
        match ch {
            'd' => match length {
                1 | 2 => digits(out, u32::from(at.day()), length),
                3 => out.push_str(&self.abbreviated_day_names[weekday(at)]),
                _ => out.push_str(&self.day_names[weekday(at)]),
            },
            'M' => {
                let month = usize::from(u8::from(at.month()) - 1);
                match length {
                    1 | 2 => digits(out, u32::from(u8::from(at.month())), length),
                    _ => {
                        let genitive = uses_genitive(chars, index, length);
                        let names = match (length == 3, genitive) {
                            (true, true) => &self.abbreviated_month_genitive_names,
                            (true, false) => &self.abbreviated_month_names,
                            (false, true) => &self.month_genitive_names,
                            (false, false) => &self.month_names,
                        };
                        out.push_str(&names[month]);
                    }
                }
            }
            'y' => {
                let year = at.year().unsigned_abs();
                match length {
                    1 => out.push_str(&(year % 100).to_string()),
                    2 => digits(out, year % 100, 2),
                    _ => {
                        let text = year.to_string();
                        for _ in text.len()..length {
                            out.push('0');
                        }
                        out.push_str(&text);
                    }
                }
            }
            'h' => {
                let hour = at.hour() % 12;
                digits(out, u32::from(if hour == 0 { 12 } else { hour }), length);
            }
            'H' => digits(out, u32::from(at.hour()), length),
            'm' => digits(out, u32::from(at.minute()), length),
            's' => digits(out, u32::from(at.second()), length),
            't' => {
                let designator = if at.hour() < 12 {
                    &self.am_designator
                } else {
                    &self.pm_designator
                };
                if length == 1 {
                    if let Some(first) = designator.chars().next() {
                        out.push(first);
                    }
                } else {
                    out.push_str(designator);
                }
            }
            ':' => {
                for _ in 0..length {
                    out.push_str(&self.time_separator);
                }
            }
            '/' => {
                for _ in 0..length {
                    out.push_str(&self.date_separator);
                }
            }
            _ => {
                for _ in 0..length {
                    out.push(ch);
                }
            }
        }
    }
}

/// .NET's `FormatDigits`: zero-padded to the run's length, but never to more than two digits.
fn digits(out: &mut String, value: u32, length: usize) {
    if length >= 2 && value < 10 {
        out.push('0');
    }
    out.push_str(&value.to_string());
}

/// Sunday = 0, as .NET's `DayOfWeek` indexes its day names.
fn weekday(at: PrimitiveDateTime) -> usize {
    usize::from(at.weekday().number_days_from_sunday())
}

/// .NET's `IsUseGenitiveForm(format, index, tokenLen, 'd')`, character for character: the nearest
/// `d` before the month run, counted back, is one or two of them; or else the nearest after it,
/// counted on, is. (Like .NET it does not skip quoted text.)
fn uses_genitive(chars: &[char], index: usize, length: usize) -> bool {
    let mut at = index;
    while at > 0 {
        at -= 1;
        if chars[at] == 'd' {
            let mut repeat = 0;
            while at > 0 && chars[at - 1] == 'd' {
                at -= 1;
                repeat += 1;
            }
            if repeat <= 1 {
                return true;
            }
            break;
        }
    }
    let mut at = index + length;
    while at < chars.len() && chars[at] != 'd' {
        at += 1;
    }
    if at < chars.len() {
        let repeat = chars[at + 1..].iter().take_while(|&&c| c == 'd').count();
        return repeat <= 1;
    }
    false
}

/// One culture's measured data, as the tables below hold it.
struct Table {
    name: &'static str,
    short_time_pattern: &'static str,
    short_date_pattern: &'static str,
    month_day_pattern: &'static str,
    long_date_pattern: &'static str,
    time_separator: &'static str,
    date_separator: &'static str,
    am_designator: &'static str,
    pm_designator: &'static str,
    month_names: [&'static str; 12],
    month_genitive_names: [&'static str; 12],
    abbreviated_month_names: [&'static str; 12],
    abbreviated_month_genitive_names: [&'static str; 12],
    day_names: [&'static str; 7],
    abbreviated_day_names: [&'static str; 7],
}

const TABLES: [&Table; 11] = [
    &INVARIANT,
    &EN_US,
    &EN_GB,
    &DE_DE,
    &ES_ES,
    &FR_FR,
    &JA_JP,
    &RU_RU,
    &SR_CYRL_RS,
    &SR_LATN_RS,
    &ZH_CN,
];

// ---- printed by the .NET oracle (`DateTimeFormatInfo` on .NET 10.0.402, ICU) -----------------

/// invariant (.NET 10 `CultureInfo.GetCultureInfo("").DateTimeFormat`).
const INVARIANT: Table = Table {
    name: "",
    short_time_pattern: "HH:mm",
    short_date_pattern: "MM/dd/yyyy",
    month_day_pattern: "MMMM dd",
    long_date_pattern: "dddd, dd MMMM yyyy",
    time_separator: ":",
    date_separator: "/",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    month_genitive_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    abbreviated_month_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    abbreviated_month_genitive_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    day_names: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    abbreviated_day_names: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
};

/// en-US (.NET 10 `CultureInfo.GetCultureInfo("en-US").DateTimeFormat`).
const EN_US: Table = Table {
    name: "en-US",
    short_time_pattern: "h:mm\u{202F}tt",
    short_date_pattern: "M/d/yyyy",
    month_day_pattern: "MMMM d",
    long_date_pattern: "dddd, MMMM d, yyyy",
    time_separator: ":",
    date_separator: "/",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    month_genitive_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    abbreviated_month_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    abbreviated_month_genitive_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    day_names: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    abbreviated_day_names: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
};

/// en-GB (.NET 10 `CultureInfo.GetCultureInfo("en-GB").DateTimeFormat`).
const EN_GB: Table = Table {
    name: "en-GB",
    short_time_pattern: "HH:mm",
    short_date_pattern: "dd/MM/yyyy",
    month_day_pattern: "d MMMM",
    long_date_pattern: "dddd, d MMMM yyyy",
    time_separator: ":",
    date_separator: "/",
    am_designator: "am",
    pm_designator: "pm",
    month_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    month_genitive_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    abbreviated_month_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    abbreviated_month_genitive_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    day_names: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    abbreviated_day_names: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
};

/// de-DE (.NET 10 `CultureInfo.GetCultureInfo("de-DE").DateTimeFormat`).
const DE_DE: Table = Table {
    name: "de-DE",
    short_time_pattern: "HH:mm",
    short_date_pattern: "dd.MM.yyyy",
    month_day_pattern: "d. MMMM",
    long_date_pattern: "dddd, d. MMMM yyyy",
    time_separator: ":",
    date_separator: ".",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "Januar",
        "Februar",
        "M\u{E4}rz",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    month_genitive_names: [
        "Januar",
        "Februar",
        "M\u{E4}rz",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    abbreviated_month_names: [
        "Jan", "Feb", "M\u{E4}r", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ],
    abbreviated_month_genitive_names: [
        "Jan.",
        "Feb.",
        "M\u{E4}rz",
        "Apr.",
        "Mai",
        "Juni",
        "Juli",
        "Aug.",
        "Sept.",
        "Okt.",
        "Nov.",
        "Dez.",
    ],
    day_names: [
        "Sonntag",
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
    ],
    abbreviated_day_names: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
};

/// es-ES (.NET 10 `CultureInfo.GetCultureInfo("es-ES").DateTimeFormat`).
const ES_ES: Table = Table {
    name: "es-ES",
    short_time_pattern: "H:mm",
    short_date_pattern: "d/M/yyyy",
    month_day_pattern: "d 'de' MMMM",
    long_date_pattern: "dddd, d 'de' MMMM 'de' yyyy",
    time_separator: ":",
    date_separator: "/",
    am_designator: "a.\u{202F}m.",
    pm_designator: "p.\u{202F}m.",
    month_names: [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    month_genitive_names: [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    abbreviated_month_names: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
    ],
    abbreviated_month_genitive_names: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
    ],
    day_names: [
        "domingo",
        "lunes",
        "martes",
        "mi\u{E9}rcoles",
        "jueves",
        "viernes",
        "s\u{E1}bado",
    ],
    abbreviated_day_names: ["dom", "lun", "mar", "mi\u{E9}", "jue", "vie", "s\u{E1}b"],
};

/// fr-FR (.NET 10 `CultureInfo.GetCultureInfo("fr-FR").DateTimeFormat`).
const FR_FR: Table = Table {
    name: "fr-FR",
    short_time_pattern: "HH:mm",
    short_date_pattern: "dd/MM/yyyy",
    month_day_pattern: "d MMMM",
    long_date_pattern: "dddd d MMMM yyyy",
    time_separator: ":",
    date_separator: "/",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "janvier",
        "f\u{E9}vrier",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "ao\u{FB}t",
        "septembre",
        "octobre",
        "novembre",
        "d\u{E9}cembre",
    ],
    month_genitive_names: [
        "janvier",
        "f\u{E9}vrier",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "ao\u{FB}t",
        "septembre",
        "octobre",
        "novembre",
        "d\u{E9}cembre",
    ],
    abbreviated_month_names: [
        "janv.",
        "f\u{E9}vr.",
        "mars",
        "avr.",
        "mai",
        "juin",
        "juil.",
        "ao\u{FB}t",
        "sept.",
        "oct.",
        "nov.",
        "d\u{E9}c.",
    ],
    abbreviated_month_genitive_names: [
        "janv.",
        "f\u{E9}vr.",
        "mars",
        "avr.",
        "mai",
        "juin",
        "juil.",
        "ao\u{FB}t",
        "sept.",
        "oct.",
        "nov.",
        "d\u{E9}c.",
    ],
    day_names: [
        "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
    ],
    abbreviated_day_names: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
};

/// ja-JP (.NET 10 `CultureInfo.GetCultureInfo("ja-JP").DateTimeFormat`).
const JA_JP: Table = Table {
    name: "ja-JP",
    short_time_pattern: "H:mm",
    short_date_pattern: "yyyy/MM/dd",
    month_day_pattern: "M\u{6708}d\u{65E5}",
    long_date_pattern: "yyyy\u{5E74}M\u{6708}d\u{65E5} dddd",
    time_separator: ":",
    date_separator: "/",
    am_designator: "\u{5348}\u{524D}",
    pm_designator: "\u{5348}\u{5F8C}",
    month_names: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    month_genitive_names: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    abbreviated_month_names: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    abbreviated_month_genitive_names: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    day_names: [
        "\u{65E5}\u{66DC}\u{65E5}",
        "\u{6708}\u{66DC}\u{65E5}",
        "\u{706B}\u{66DC}\u{65E5}",
        "\u{6C34}\u{66DC}\u{65E5}",
        "\u{6728}\u{66DC}\u{65E5}",
        "\u{91D1}\u{66DC}\u{65E5}",
        "\u{571F}\u{66DC}\u{65E5}",
    ],
    abbreviated_day_names: [
        "\u{65E5}", "\u{6708}", "\u{706B}", "\u{6C34}", "\u{6728}", "\u{91D1}", "\u{571F}",
    ],
};

/// ru-RU (.NET 10 `CultureInfo.GetCultureInfo("ru-RU").DateTimeFormat`).
const RU_RU: Table = Table {
    name: "ru-RU",
    short_time_pattern: "HH:mm",
    short_date_pattern: "dd.MM.yyyy",
    month_day_pattern: "d MMMM",
    long_date_pattern: "dddd, d MMMM yyyy '\u{433}'.",
    time_separator: ":",
    date_separator: ".",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "\u{44F}\u{43D}\u{432}\u{430}\u{440}\u{44C}",
        "\u{444}\u{435}\u{432}\u{440}\u{430}\u{43B}\u{44C}",
        "\u{43C}\u{430}\u{440}\u{442}",
        "\u{430}\u{43F}\u{440}\u{435}\u{43B}\u{44C}",
        "\u{43C}\u{430}\u{439}",
        "\u{438}\u{44E}\u{43D}\u{44C}",
        "\u{438}\u{44E}\u{43B}\u{44C}",
        "\u{430}\u{432}\u{433}\u{443}\u{441}\u{442}",
        "\u{441}\u{435}\u{43D}\u{442}\u{44F}\u{431}\u{440}\u{44C}",
        "\u{43E}\u{43A}\u{442}\u{44F}\u{431}\u{440}\u{44C}",
        "\u{43D}\u{43E}\u{44F}\u{431}\u{440}\u{44C}",
        "\u{434}\u{435}\u{43A}\u{430}\u{431}\u{440}\u{44C}",
    ],
    month_genitive_names: [
        "\u{44F}\u{43D}\u{432}\u{430}\u{440}\u{44F}",
        "\u{444}\u{435}\u{432}\u{440}\u{430}\u{43B}\u{44F}",
        "\u{43C}\u{430}\u{440}\u{442}\u{430}",
        "\u{430}\u{43F}\u{440}\u{435}\u{43B}\u{44F}",
        "\u{43C}\u{430}\u{44F}",
        "\u{438}\u{44E}\u{43D}\u{44F}",
        "\u{438}\u{44E}\u{43B}\u{44F}",
        "\u{430}\u{432}\u{433}\u{443}\u{441}\u{442}\u{430}",
        "\u{441}\u{435}\u{43D}\u{442}\u{44F}\u{431}\u{440}\u{44F}",
        "\u{43E}\u{43A}\u{442}\u{44F}\u{431}\u{440}\u{44F}",
        "\u{43D}\u{43E}\u{44F}\u{431}\u{440}\u{44F}",
        "\u{434}\u{435}\u{43A}\u{430}\u{431}\u{440}\u{44F}",
    ],
    abbreviated_month_names: [
        "\u{44F}\u{43D}\u{432}.",
        "\u{444}\u{435}\u{432}\u{440}.",
        "\u{43C}\u{430}\u{440}\u{442}",
        "\u{430}\u{43F}\u{440}.",
        "\u{43C}\u{430}\u{439}",
        "\u{438}\u{44E}\u{43D}\u{44C}",
        "\u{438}\u{44E}\u{43B}\u{44C}",
        "\u{430}\u{432}\u{433}.",
        "\u{441}\u{435}\u{43D}\u{442}.",
        "\u{43E}\u{43A}\u{442}.",
        "\u{43D}\u{43E}\u{44F}\u{431}.",
        "\u{434}\u{435}\u{43A}.",
    ],
    abbreviated_month_genitive_names: [
        "\u{44F}\u{43D}\u{432}.",
        "\u{444}\u{435}\u{432}\u{440}.",
        "\u{43C}\u{430}\u{440}\u{442}\u{430}",
        "\u{430}\u{43F}\u{440}.",
        "\u{43C}\u{430}\u{44F}",
        "\u{438}\u{44E}\u{43D}\u{44F}",
        "\u{438}\u{44E}\u{43B}\u{44F}",
        "\u{430}\u{432}\u{433}.",
        "\u{441}\u{435}\u{43D}\u{442}.",
        "\u{43E}\u{43A}\u{442}.",
        "\u{43D}\u{43E}\u{44F}\u{431}.",
        "\u{434}\u{435}\u{43A}.",
    ],
    day_names: [
        "\u{432}\u{43E}\u{441}\u{43A}\u{440}\u{435}\u{441}\u{435}\u{43D}\u{44C}\u{435}",
        "\u{43F}\u{43E}\u{43D}\u{435}\u{434}\u{435}\u{43B}\u{44C}\u{43D}\u{438}\u{43A}",
        "\u{432}\u{442}\u{43E}\u{440}\u{43D}\u{438}\u{43A}",
        "\u{441}\u{440}\u{435}\u{434}\u{430}",
        "\u{447}\u{435}\u{442}\u{432}\u{435}\u{440}\u{433}",
        "\u{43F}\u{44F}\u{442}\u{43D}\u{438}\u{446}\u{430}",
        "\u{441}\u{443}\u{431}\u{431}\u{43E}\u{442}\u{430}",
    ],
    abbreviated_day_names: [
        "\u{412}\u{441}",
        "\u{41F}\u{43D}",
        "\u{412}\u{442}",
        "\u{421}\u{440}",
        "\u{427}\u{442}",
        "\u{41F}\u{442}",
        "\u{421}\u{431}",
    ],
};

/// sr-Cyrl-RS (.NET 10 `CultureInfo.GetCultureInfo("sr-Cyrl-RS").DateTimeFormat`).
const SR_CYRL_RS: Table = Table {
    name: "sr-Cyrl-RS",
    short_time_pattern: "HH:mm",
    short_date_pattern: "d. M. yyyy.",
    month_day_pattern: "d. MMMM",
    long_date_pattern: "dddd, d. MMMM yyyy.",
    time_separator: ":",
    date_separator: ". ",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "\u{458}\u{430}\u{43D}\u{443}\u{430}\u{440}",
        "\u{444}\u{435}\u{431}\u{440}\u{443}\u{430}\u{440}",
        "\u{43C}\u{430}\u{440}\u{442}",
        "\u{430}\u{43F}\u{440}\u{438}\u{43B}",
        "\u{43C}\u{430}\u{458}",
        "\u{458}\u{443}\u{43D}",
        "\u{458}\u{443}\u{43B}",
        "\u{430}\u{432}\u{433}\u{443}\u{441}\u{442}",
        "\u{441}\u{435}\u{43F}\u{442}\u{435}\u{43C}\u{431}\u{430}\u{440}",
        "\u{43E}\u{43A}\u{442}\u{43E}\u{431}\u{430}\u{440}",
        "\u{43D}\u{43E}\u{432}\u{435}\u{43C}\u{431}\u{430}\u{440}",
        "\u{434}\u{435}\u{446}\u{435}\u{43C}\u{431}\u{430}\u{440}",
    ],
    month_genitive_names: [
        "\u{458}\u{430}\u{43D}\u{443}\u{430}\u{440}",
        "\u{444}\u{435}\u{431}\u{440}\u{443}\u{430}\u{440}",
        "\u{43C}\u{430}\u{440}\u{442}",
        "\u{430}\u{43F}\u{440}\u{438}\u{43B}",
        "\u{43C}\u{430}\u{458}",
        "\u{458}\u{443}\u{43D}",
        "\u{458}\u{443}\u{43B}",
        "\u{430}\u{432}\u{433}\u{443}\u{441}\u{442}",
        "\u{441}\u{435}\u{43F}\u{442}\u{435}\u{43C}\u{431}\u{430}\u{440}",
        "\u{43E}\u{43A}\u{442}\u{43E}\u{431}\u{430}\u{440}",
        "\u{43D}\u{43E}\u{432}\u{435}\u{43C}\u{431}\u{430}\u{440}",
        "\u{434}\u{435}\u{446}\u{435}\u{43C}\u{431}\u{430}\u{440}",
    ],
    abbreviated_month_names: [
        "\u{458}\u{430}\u{43D}",
        "\u{444}\u{435}\u{431}",
        "\u{43C}\u{430}\u{440}",
        "\u{430}\u{43F}\u{440}",
        "\u{43C}\u{430}\u{458}",
        "\u{458}\u{443}\u{43D}",
        "\u{458}\u{443}\u{43B}",
        "\u{430}\u{432}\u{433}",
        "\u{441}\u{435}\u{43F}",
        "\u{43E}\u{43A}\u{442}",
        "\u{43D}\u{43E}\u{432}",
        "\u{434}\u{435}\u{446}",
    ],
    abbreviated_month_genitive_names: [
        "\u{458}\u{430}\u{43D}",
        "\u{444}\u{435}\u{431}",
        "\u{43C}\u{430}\u{440}",
        "\u{430}\u{43F}\u{440}",
        "\u{43C}\u{430}\u{458}",
        "\u{458}\u{443}\u{43D}",
        "\u{458}\u{443}\u{43B}",
        "\u{430}\u{432}\u{433}",
        "\u{441}\u{435}\u{43F}",
        "\u{43E}\u{43A}\u{442}",
        "\u{43D}\u{43E}\u{432}",
        "\u{434}\u{435}\u{446}",
    ],
    day_names: [
        "\u{43D}\u{435}\u{434}\u{435}\u{459}\u{430}",
        "\u{43F}\u{43E}\u{43D}\u{435}\u{434}\u{435}\u{459}\u{430}\u{43A}",
        "\u{443}\u{442}\u{43E}\u{440}\u{430}\u{43A}",
        "\u{441}\u{440}\u{435}\u{434}\u{430}",
        "\u{447}\u{435}\u{442}\u{432}\u{440}\u{442}\u{430}\u{43A}",
        "\u{43F}\u{435}\u{442}\u{430}\u{43A}",
        "\u{441}\u{443}\u{431}\u{43E}\u{442}\u{430}",
    ],
    abbreviated_day_names: [
        "\u{43D}\u{435}\u{434}",
        "\u{43F}\u{43E}\u{43D}",
        "\u{443}\u{442}\u{43E}",
        "\u{441}\u{440}\u{435}",
        "\u{447}\u{435}\u{442}",
        "\u{43F}\u{435}\u{442}",
        "\u{441}\u{443}\u{431}",
    ],
};

/// sr-Latn-RS (.NET 10 `CultureInfo.GetCultureInfo("sr-Latn-RS").DateTimeFormat`).
const SR_LATN_RS: Table = Table {
    name: "sr-Latn-RS",
    short_time_pattern: "HH:mm",
    short_date_pattern: "d. M. yyyy.",
    month_day_pattern: "d. MMMM",
    long_date_pattern: "dddd, d. MMMM yyyy.",
    time_separator: ":",
    date_separator: ". ",
    am_designator: "AM",
    pm_designator: "PM",
    month_names: [
        "januar",
        "februar",
        "mart",
        "april",
        "maj",
        "jun",
        "jul",
        "avgust",
        "septembar",
        "oktobar",
        "novembar",
        "decembar",
    ],
    month_genitive_names: [
        "januar",
        "februar",
        "mart",
        "april",
        "maj",
        "jun",
        "jul",
        "avgust",
        "septembar",
        "oktobar",
        "novembar",
        "decembar",
    ],
    abbreviated_month_names: [
        "jan", "feb", "mar", "apr", "maj", "jun", "jul", "avg", "sep", "okt", "nov", "dec",
    ],
    abbreviated_month_genitive_names: [
        "jan", "feb", "mar", "apr", "maj", "jun", "jul", "avg", "sep", "okt", "nov", "dec",
    ],
    day_names: [
        "nedelja",
        "ponedeljak",
        "utorak",
        "sreda",
        "\u{10D}etvrtak",
        "petak",
        "subota",
    ],
    abbreviated_day_names: ["ned", "pon", "uto", "sre", "\u{10D}et", "pet", "sub"],
};

/// zh-CN (.NET 10 `CultureInfo.GetCultureInfo("zh-CN").DateTimeFormat`).
const ZH_CN: Table = Table {
    name: "zh-CN",
    short_time_pattern: "HH:mm",
    short_date_pattern: "yyyy/M/d",
    month_day_pattern: "M\u{6708}d\u{65E5}",
    long_date_pattern: "yyyy\u{5E74}M\u{6708}d\u{65E5} dddd",
    time_separator: ":",
    date_separator: "/",
    am_designator: "\u{4E0A}\u{5348}",
    pm_designator: "\u{4E0B}\u{5348}",
    month_names: [
        "\u{4E00}\u{6708}",
        "\u{4E8C}\u{6708}",
        "\u{4E09}\u{6708}",
        "\u{56DB}\u{6708}",
        "\u{4E94}\u{6708}",
        "\u{516D}\u{6708}",
        "\u{4E03}\u{6708}",
        "\u{516B}\u{6708}",
        "\u{4E5D}\u{6708}",
        "\u{5341}\u{6708}",
        "\u{5341}\u{4E00}\u{6708}",
        "\u{5341}\u{4E8C}\u{6708}",
    ],
    month_genitive_names: [
        "\u{4E00}\u{6708}",
        "\u{4E8C}\u{6708}",
        "\u{4E09}\u{6708}",
        "\u{56DB}\u{6708}",
        "\u{4E94}\u{6708}",
        "\u{516D}\u{6708}",
        "\u{4E03}\u{6708}",
        "\u{516B}\u{6708}",
        "\u{4E5D}\u{6708}",
        "\u{5341}\u{6708}",
        "\u{5341}\u{4E00}\u{6708}",
        "\u{5341}\u{4E8C}\u{6708}",
    ],
    abbreviated_month_names: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    abbreviated_month_genitive_names: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    day_names: [
        "\u{661F}\u{671F}\u{65E5}",
        "\u{661F}\u{671F}\u{4E00}",
        "\u{661F}\u{671F}\u{4E8C}",
        "\u{661F}\u{671F}\u{4E09}",
        "\u{661F}\u{671F}\u{56DB}",
        "\u{661F}\u{671F}\u{4E94}",
        "\u{661F}\u{671F}\u{516D}",
    ],
    abbreviated_day_names: [
        "\u{5468}\u{65E5}",
        "\u{5468}\u{4E00}",
        "\u{5468}\u{4E8C}",
        "\u{5468}\u{4E09}",
        "\u{5468}\u{56DB}",
        "\u{5468}\u{4E94}",
        "\u{5468}\u{516D}",
    ],
};
