use rustc_hash::FxHashSet;

pub struct Finding {
    pub path: String,
    pub reason: String,
}

const FORBIDDEN_EXT: &[&str] = &[
    "sfc",
    "smc",
    "swc",
    "fig",
    "ips",
    "bps",
    "state",
    "ssdstate",
    "ssdreplay",
    "srm",
    "sav",
    "cdl",
    "mlb",
    "log",
    "exe",
    "dll",
    "csv",
    "gpr",
    "gzf",
    "mss",
    "mst",
    "bak",
    "orig",
    "tmp",
];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];
const WINDOW: usize = 16;

pub fn check_paths(paths: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in paths {
        let lower = p.to_ascii_lowercase();
        let segments: Vec<&str> = lower.split('/').collect();

        let reason = {
            // Check for forbidden segment names
            let mut found_reason = None;
            for segment in &segments {
                if *segment == ".work" || *segment == "target" || *segment == "cache" {
                    found_reason = Some("scratch or tool database directory");
                    break;
                }
                if segment.rsplit_once('.').is_some_and(|(_, ext)| ext == "rep") {
                    found_reason = Some("scratch or tool database directory");
                    break;
                }
            }

            if found_reason.is_some() {
                found_reason
            } else {
                let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
                if FORBIDDEN_EXT.contains(&ext) || lower.ends_with('~') {
                    Some("ROM-derived or scratch file type")
                } else if IMAGE_EXT.contains(&ext) {
                    // Images are allowed in assets/ (approved art) and docs/media/ (the pictures
                    // of the README; a development checkout's .gitignore keeps screenshots out)
                    let approved = segments[0] == "assets"
                        || (segments.len() > 2 && segments[0] == "docs" && segments[1] == "media");
                    if approved {
                        None
                    } else {
                        Some("image outside assets/ and docs/media/ (screenshots are ROM-derived)")
                    }
                } else {
                    None
                }
            }
        };

        if let Some(r) = reason {
            out.push(Finding { path: p.clone(), reason: r.to_owned() });
        }
    }
    out
}

/// Third-party files vetted by hand and exempt from the ROM scan, by path and CRC32 of their exact
/// content: binary fonts (OFL) contain structured 16-byte runs (UTF-16 name tables, ascending
/// glyph indices) that can also occur in the ROM. Any other content at these paths is scanned as
/// usual, so replacing a font means vetting it again and updating its CRC here.
pub const VETTED: &[(&str, u32)] = &[
    ("assets/fonts/IBMPlexSans-Regular.ttf", 0x7359_01B3),
    ("assets/fonts/IBMPlexSans-SemiBold.ttf", 0xC69D_420D),
    ("assets/fonts/JetBrainsMono-Regular.ttf", 0x855C_E738),
    // DotGothic16 (SIL OFL 1.1, Fontworks): Google Fonts' copy of the upstream commit e44ca7b.
    ("assets/fonts/DotGothic16-Regular.ttf", 0x3225_1781),
    // Share Tech Mono (SIL OFL 1.1, Carrois Type Design): Google Fonts' copy; the HD text of the game.
    ("assets/fonts/ShareTechMono-Regular.ttf", 0x1D7A_0035),
];

/// Whether `data` is the vetted content of `path` ([`VETTED`]).
#[must_use]
pub fn is_vetted(path: &str, data: &[u8]) -> bool {
    VETTED.iter().any(|&(vetted, crc)| vetted == path && ssd_rom::crc32(data) == crc)
}

fn high_entropy(w: &[u8]) -> bool {
    let mut seen = [false; 256];
    let mut distinct = 0;
    for &b in w {
        if !seen[b as usize] {
            seen[b as usize] = true;
            distinct += 1;
        }
    }
    distinct >= 4
}

pub struct RomIndex {
    windows: FxHashSet<[u8; WINDOW]>,
}

impl RomIndex {
    pub fn new(rom: &[u8]) -> Self {
        let mut windows = FxHashSet::default();
        for w in rom.windows(WINDOW) {
            if high_entropy(w) {
                windows.insert(w.try_into().expect("window size"));
            }
        }
        Self { windows }
    }

    pub fn find_run(&self, data: &[u8]) -> Option<usize> {
        data.windows(WINDOW).position(|w| {
            high_entropy(w)
                && self.windows.contains::<[u8; WINDOW]>(&w.try_into().expect("window size"))
        })
    }

    /// Whether the ROM yielded no window to look for (empty, tiny or low-entropy): scanning
    /// against such an index would prove nothing.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Line where the first byte-literal run of `text` that contains ROM bytes starts, looking at
    /// the runs with comments as breaks first and then at the runs that see through comments.
    pub fn find_literal(&self, text: &[u8]) -> Option<usize> {
        [Comments::Break, Comments::Skip].into_iter().find_map(|comments| {
            literal_runs(text, comments)
                .into_iter()
                .find(|run| self.find_run(&run.bytes).is_some())
                .map(|run| line_at(text, run.start))
        })
    }
}

/// Extensions of the tracked files whose byte literals are scanned.
const LITERAL_EXT: &[&str] = &["rs", "toml", "md", "txt"];

/// Whether `path` is a source or text file that may carry ROM tables pasted as literals.
pub fn is_literal_scanned(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.rsplit_once('.').is_some_and(|(_, ext)| LITERAL_EXT.contains(&ext))
}

/// A run of byte literals found in a source file, with the offset where it starts.
#[derive(Debug, PartialEq, Eq)]
pub struct LiteralRun {
    pub start: usize,
    pub bytes: Vec<u8>,
}

/// How [`literal_runs`] treats `//` and `/* */` comments between literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comments {
    /// A comment ends the run, like any other text.
    Break,
    /// A comment is skipped, unread, and the run goes on after it.
    Skip,
}

enum Token {
    /// Bytes spelled by a `0xNN` literal or a `b"..."` string.
    Bytes(Vec<u8>),
    /// A literal that is not a byte (`0x1234`, `0x12usize`): it ends the current run.
    Other,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Characters allowed between the literals of one run: whitespace, commas, `&` and the brackets of
/// nested tables, `|` of markdown table rows, so that a table pasted row by row is one run.
fn is_separator(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(b, b',' | b';' | b'&' | b'|' | b'[' | b']' | b'(' | b')' | b'{' | b'}')
}

/// `0xN`, `0xNN`, optionally with `_` digit separators and a `u8` suffix.
fn hex_token(text: &[u8], start: usize) -> Option<(Token, usize)> {
    if !text[start..].starts_with(b"0x") || (start > 0 && is_ident_byte(text[start - 1])) {
        return None;
    }
    let digits_start = start + 2;
    let digits_end = digits_start
        + text[digits_start..].iter().take_while(|&&b| b.is_ascii_hexdigit() || b == b'_').count();
    let end = digits_end + text[digits_end..].iter().take_while(|&&b| is_ident_byte(b)).count();
    let digits: String = text[digits_start..digits_end]
        .iter()
        .filter(|&&b| b != b'_')
        .map(|&b| char::from(b))
        .collect();
    if digits.is_empty() {
        return None;
    }
    let suffix = &text[digits_end..end];
    if digits.len() > 2 || !(suffix.is_empty() || suffix == b"u8") {
        return Some((Token::Other, end));
    }
    let value = u8::from_str_radix(&digits, 16).expect("one or two hex digits");
    Some((Token::Bytes(vec![value]), end))
}

/// `b"..."` with `\xNN`, `\n`, `\r`, `\t`, `\0`, `\\`, `\'`, `\"` escapes and line continuations.
fn byte_string_token(text: &[u8], start: usize) -> Option<(Token, usize)> {
    if !text[start..].starts_with(b"b\"") || (start > 0 && is_ident_byte(text[start - 1])) {
        return None;
    }
    let mut bytes = Vec::new();
    let mut i = start + 2;
    while i < text.len() {
        match text[i] {
            b'"' => {
                i += 1;
                break;
            }
            b'\\' => {
                i += 1;
                match text.get(i) {
                    Some(b'x') => {
                        let digits = text.get(i + 1..i + 3).unwrap_or_default();
                        if digits.len() == 2 && digits.iter().all(u8::is_ascii_hexdigit) {
                            let hex = std::str::from_utf8(digits).expect("ASCII hex digits");
                            bytes.push(u8::from_str_radix(hex, 16).expect("two hex digits"));
                            i += 2;
                        }
                    }
                    Some(b'n') => bytes.push(b'\n'),
                    Some(b'r') => bytes.push(b'\r'),
                    Some(b't') => bytes.push(b'\t'),
                    Some(b'0') => bytes.push(0),
                    Some(&c @ (b'\\' | b'\'' | b'"')) => bytes.push(c),
                    Some(b'\n' | b'\r') => {
                        while text.get(i + 1).is_some_and(u8::is_ascii_whitespace) {
                            i += 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            c => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    Some((Token::Bytes(bytes), i))
}

/// 1-based line of byte `offset` in `text`: one more than the newlines before it.
fn line_at(text: &[u8], offset: usize) -> usize {
    text[..offset].split(|&b| b == b'\n').count()
}

/// End of the `//` or `/* */` comment starting at `start`, if one does: the next newline (which
/// stays whitespace) or the end of `text` for `//`, past the closing `*/` or the end of `text` for
/// `/*`. The contents are not read, nested block comments are not tracked.
fn comment_end(text: &[u8], start: usize) -> Option<usize> {
    let rest = &text[start..];
    if rest.starts_with(b"//") {
        Some(rest.iter().position(|&b| b == b'\n').map_or(text.len(), |n| start + n))
    } else if rest.starts_with(b"/*") {
        Some(rest[2..].windows(2).position(|w| w == b"*/").map_or(text.len(), |n| start + n + 4))
    } else {
        None
    }
}

/// Collects the runs of byte literals (`0xNN` and `b"..."`) in `text`.
///
/// A run is a maximal sequence of byte literals separated only by whitespace, commas, semicolons,
/// `&`, `|` and brackets. Decimal numbers, wider hex literals and identifiers end a run, and so
/// do comments with [`Comments::Break`]; with [`Comments::Skip`] a `//` or `/* */` comment between
/// two literals is jumped over unread and the run goes on.
pub fn literal_runs(text: &[u8], comments: Comments) -> Vec<LiteralRun> {
    let mut runs = Vec::new();
    // Start offset and bytes of the run being collected.
    let mut open: Option<(usize, Vec<u8>)> = None;
    let mut i = 0;
    while i < text.len() {
        if comments == Comments::Skip
            && let Some(end) = comment_end(text, i)
        {
            i = end;
            continue;
        }
        match hex_token(text, i).or_else(|| byte_string_token(text, i)) {
            Some((Token::Bytes(bytes), end)) => {
                open.get_or_insert_with(|| (i, Vec::new())).1.extend(bytes);
                i = end;
            }
            Some((Token::Other, end)) => {
                close_run(&mut open, &mut runs);
                i = end;
            }
            None => {
                if !is_separator(text[i]) {
                    close_run(&mut open, &mut runs);
                }
                i += 1;
            }
        }
    }
    close_run(&mut open, &mut runs);
    runs
}

fn close_run(open: &mut Option<(usize, Vec<u8>)>, runs: &mut Vec<LiteralRun>) {
    if let Some((start, bytes)) = open.take() {
        runs.push(LiteralRun { start, bytes });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn flags_forbidden_files() {
        let f = check_paths(&s(&[
            "crates/ssd-rom/src/lib.rs",
            "assets/cards/goku.png",
            "docs/shot.png",
            "docs/media/cmp-a.png",
            "docs/media.png",
            "game.sfc",
            ".work/x.txt",
            "re/labels.mlb",
            "notes/run.log",
            "crates/x/.work/note.txt",
            "target/out.txt",
            "cache/rom/x.dat",
            "ghidra/proj.rep/x.db",
            "UP.PNG",
            "notes/a.tmp",
            "src/lib.rs~",
            "tools/re/Mesen.exe",
            "tools/re/MesenCore.dll",
            "tools/re/out/exec.csv",
            "tools/re/prgrom.cdl",
            "tools/re/dbz.mlb",
            "tools/re/trace.log",
            "replays/battle.ssdreplay",
        ]));
        let flagged: Vec<_> = f.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(
            flagged,
            [
                "docs/shot.png",
                "docs/media.png",
                "game.sfc",
                ".work/x.txt",
                "re/labels.mlb",
                "notes/run.log",
                "crates/x/.work/note.txt",
                "target/out.txt",
                "cache/rom/x.dat",
                "ghidra/proj.rep/x.db",
                "UP.PNG",
                "notes/a.tmp",
                "src/lib.rs~",
                "tools/re/Mesen.exe",
                "tools/re/MesenCore.dll",
                "tools/re/out/exec.csv",
                "tools/re/prgrom.cdl",
                "tools/re/dbz.mlb",
                "tools/re/trace.log",
                "replays/battle.ssdreplay",
            ]
        );
    }

    #[test]
    fn finds_rom_runs_but_ignores_low_entropy() {
        let rom: Vec<u8> = (0..4096_u32).map(|i| (i * 7 + i / 13) as u8).collect();
        let index = RomIndex::new(&rom);
        let mut file = b"fn main() {}\n".to_vec();
        file.extend_from_slice(&rom[1000..1020]);
        assert_eq!(index.find_run(&file), Some(13));
        let zeros = vec![0_u8; 64];
        let index2 = RomIndex::new(&zeros);
        assert_eq!(index2.find_run(&zeros), None);
        assert_eq!(index.find_run(b"completely unrelated text here"), None);

        // Test 15-byte run is not found
        let mut file_15 = b"fn main() {}\n".to_vec();
        file_15.extend_from_slice(&rom[1000..1015]); // Only 15 bytes
        assert_eq!(index.find_run(&file_15), None);

        // Test 16-byte run is found
        let mut file_16 = b"fn main() {}\n".to_vec();
        file_16.extend_from_slice(&rom[1000..1016]); // Exactly 16 bytes
        assert_eq!(index.find_run(&file_16), Some(13));

        // Test 3 distinct byte values ignored
        let rom_3_distinct = vec![0_u8, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0];
        let index_3 = RomIndex::new(&rom_3_distinct);
        assert_eq!(index_3.find_run(&rom_3_distinct), None);

        // Test 4 distinct byte values counted
        let rom_4_distinct = vec![0_u8, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3];
        let index_4 = RomIndex::new(&rom_4_distinct);
        assert_eq!(index_4.find_run(&rom_4_distinct), Some(0));
    }

    #[test]
    fn only_the_vetted_content_of_a_vetted_path_is_exempt() {
        let (path, _) = VETTED[0];
        assert!(!is_vetted(path, b"not the font"), "other content at a vetted path is scanned");
        assert!(!is_vetted("assets/fonts/Other.ttf", b""), "unlisted paths are scanned");
        let font = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../",
            "assets/fonts/IBMPlexSans-Regular.ttf"
        ));
        if let Ok(font) = font {
            assert!(is_vetted("assets/fonts/IBMPlexSans-Regular.ttf", &font));
            assert!(
                !is_vetted("assets/fonts/JetBrainsMono-Regular.ttf", &font),
                "path and content go together"
            );
        }
    }

    /// Deterministic pseudo-ROM with plenty of distinct bytes.
    fn fake_rom() -> Vec<u8> {
        (0..4096_u32).map(|i| (i * 7 + i / 13) as u8).collect()
    }

    fn hex_list(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("0x{b:02X},")).collect::<Vec<_>>().join(" ")
    }

    fn byte_string(bytes: &[u8]) -> String {
        let escaped = bytes.iter().map(|b| format!("\\x{b:02x}")).collect::<Vec<_>>().concat();
        format!("b\"{escaped}\"")
    }

    #[test]
    fn literal_scan_applies_to_source_and_text_files() {
        for p in ["a/b.rs", "Cargo.toml", "docs/x.md", "notes.txt", "A/B.RS", "re/symbols.TOML"] {
            assert!(is_literal_scanned(p), "{p}");
        }
        for p in ["Cargo.lock", "a.png", "x.rs.bak", "rs", "docs/readme"] {
            assert!(!is_literal_scanned(p), "{p}");
        }
    }

    #[test]
    fn literal_runs_parses_hex_bytes() {
        let runs = literal_runs(b"const T: [u8; 4] = [0x01, 0xAb,0x7f\n\t0x00];", Comments::Break);
        assert_eq!(runs, [LiteralRun { start: 20, bytes: vec![0x01, 0xAB, 0x7F, 0x00] }]);
        // One digit, a `u8` suffix and digit separators are still bytes.
        let runs = literal_runs(b"[0x1, 0x2u8, 0x0F_u8, 0xA_B]", Comments::Break);
        assert_eq!(runs[0].bytes, [0x01, 0x02, 0x0F, 0xAB]);
    }

    #[test]
    fn literal_runs_break_on_everything_that_is_not_a_byte() {
        // Wider hex literals, decimals, identifiers and comments split runs.
        let text = b"0x01, 0x02, 0x1234, 0x03, 7, 0x04, name, 0x05, // note\n0x06";
        let runs = literal_runs(text, Comments::Break);
        let got: Vec<&[u8]> = runs.iter().map(|r| r.bytes.as_slice()).collect();
        assert_eq!(got, [&[1_u8, 2][..], &[3], &[4], &[5], &[6]]);
        // `0x` glued to an identifier or a wider suffix is not a byte literal.
        assert_eq!(literal_runs(b"foo0x12, 0x12usize, 0x12u16", Comments::Break), []);
        assert_eq!(literal_runs(b"[12, 34, 56, 78, 90]", Comments::Break), []);
    }

    #[test]
    fn literal_runs_join_rows_of_nested_tables() {
        let runs = literal_runs(b"[[0x01, 0x02],\n [0x03, 0x04]]; ({0x05})", Comments::Break);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bytes, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn literal_runs_parse_byte_strings() {
        let text = b"let a = b\"\\x01\\x02AB\\n\\0\\\\\\\"\\x7f\"; let c = b\"x\\\n      y\";";
        let runs = literal_runs(text, Comments::Break);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].bytes, [0x01, 0x02, b'A', b'B', b'\n', 0, b'\\', b'"', 0x7F]);
        // A backslash-newline continuation skips the next line's indentation.
        assert_eq!(runs[1].bytes, b"xy");
        // Adjacent byte literals of different kinds form one run.
        let runs = literal_runs(b"[0x01, b\"\\x02\\x03\", 0x04]", Comments::Break);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bytes, [1, 2, 3, 4]);
        // A `b"` inside an identifier is not a byte string.
        assert_eq!(literal_runs(b"ab\"\\x01\"", Comments::Break), []);
    }

    fn position(text: &[u8], needle: &[u8]) -> usize {
        text.windows(needle.len()).position(|w| w == needle).expect("needle in text")
    }

    #[test]
    fn literal_runs_report_start_offsets_and_lines() {
        let text =
            b"fn f() {}\n\nconst A: [u8; 2] = [0x01, 0x02];\nstatic B: &[u8] = b\"\\x03\";\n";
        let starts: Vec<usize> =
            literal_runs(text, Comments::Break).iter().map(|r| r.start).collect();
        assert_eq!(starts, [position(text, b"0x01"), position(text, b"b\"")]);
        let lines: Vec<usize> = starts.iter().map(|&start| line_at(text, start)).collect();
        assert_eq!(lines, [3, 4]);
        assert_eq!(line_at(b"", 0), 1);
        assert_eq!(line_at(b"a\nb", 2), 2);
    }

    #[test]
    fn literal_runs_can_see_through_comments() {
        // Skip: comments neither end a run nor contribute bytes.
        let text = b"0x01, 0x02, 0x1234, 0x03, 7, 0x04, name, 0x05, // note\n0x06, /* 0x07 */ 0x08";
        let runs = literal_runs(text, Comments::Skip);
        let got: Vec<&[u8]> = runs.iter().map(|r| r.bytes.as_slice()).collect();
        assert_eq!(got, [&[1_u8, 2][..], &[3], &[4], &[5, 6, 8]]);
        // Break: the comment is text like any other.
        let runs = literal_runs(text, Comments::Break);
        let got: Vec<&[u8]> = runs.iter().map(|r| r.bytes.as_slice()).collect();
        assert_eq!(got, [&[1_u8, 2][..], &[3], &[4], &[5], &[6], &[7], &[8]]);
        // Hex in a comment is not read: no stray byte in `0xA9, 0x00, // LDA #0x00`.
        let text = b"0xA9, 0x00, // LDA #0x00\n0xFF, /* STA\n0x2100 */ 0x8D";
        let runs = literal_runs(text, Comments::Skip);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bytes, [0xA9, 0x00, 0xFF, 0x8D]);
        // A `//` or `/*` inside a byte string belongs to the string.
        let runs = literal_runs(b"[0x01, b\"a//b/*c\\x02\", 0x03]", Comments::Skip);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bytes, b"\x01a//b/*c\x02\x03");
        // Unterminated comments run to the end of the text.
        let runs = literal_runs(b"[0x01, /* 0x02, 0x03", Comments::Skip);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bytes, [1]);
        let runs = literal_runs(b"[0x01, // 0x02, 0x03", Comments::Skip);
        assert_eq!(runs[0].bytes, [1]);
        let runs = literal_runs(b"[0x01, /", Comments::Skip);
        assert_eq!(runs[0].bytes, [1]);
        assert_eq!(literal_runs(b"/*", Comments::Skip), []);
    }

    #[test]
    fn literal_runs_join_rows_across_ampersands_and_pipes() {
        for mode in [Comments::Break, Comments::Skip] {
            let runs = literal_runs(b"[&[0x01, 0x02], &[0x03]]", mode);
            assert_eq!(runs.len(), 1, "{mode:?}");
            assert_eq!(runs[0].bytes, [1, 2, 3], "{mode:?}");
            // Rows of a markdown table.
            let runs = literal_runs(b"| 0x01 | 0x02 |\n| 0x03 | 0x04 |", mode);
            assert_eq!(runs.len(), 1, "{mode:?}");
            assert_eq!(runs[0].bytes, [1, 2, 3, 4], "{mode:?}");
        }
    }

    #[test]
    fn finds_rom_bytes_pasted_as_hex_literals() {
        let rom = fake_rom();
        let index = RomIndex::new(&rom);
        let src = |n: usize| {
            let table = hex_list(&rom[1000..1000 + n]);
            format!("fn x() {{}}\n\nconst T: &[u8] = &[\n    {table}\n];\n")
        };
        // 20 ROM bytes are flagged and the line of the run is reported.
        assert_eq!(index.find_literal(src(20).as_bytes()), Some(4));
        assert_eq!(index.find_literal(src(16).as_bytes()), Some(4));
        // 15 are below the window and are not.
        assert_eq!(index.find_literal(src(15).as_bytes()), None);
        // Rows of a nested table are one run.
        let rows = format!("[[{}], [{}]]", hex_list(&rom[2000..2008]), hex_list(&rom[2008..2016]));
        assert_eq!(index.find_literal(rows.as_bytes()), Some(1));
        // Bare numbers are not read as bytes: the same 20 values in decimal pass.
        let decimal = rom[1000..1020].iter().map(|b| format!("{b}, ")).collect::<Vec<_>>().concat();
        assert_eq!(index.find_literal(decimal.as_bytes()), None);
        // Literals separated by other tokens do not form a run.
        let broken =
            rom[1000..1020].iter().map(|b| format!("0x{b:02X}, x, ")).collect::<Vec<_>>().concat();
        assert_eq!(index.find_literal(broken.as_bytes()), None);
    }

    #[test]
    fn finds_rom_bytes_pasted_with_row_comments() {
        let rom = fake_rom();
        let index = RomIndex::new(&rom);
        let table = |n: usize, row_comment: &dyn Fn(usize) -> String| {
            let rows = rom[1000..1000 + n]
                .chunks(4)
                .enumerate()
                .map(|(row, bytes)| row_comment(row).replace("{}", &hex_list(bytes)))
                .collect::<Vec<_>>()
                .concat();
            format!("const T: &[u8] = &[\n{rows}];\n")
        };
        // The style of our own fixtures: one row per line, each with a `//` comment.
        let line_comments = |row: usize| format!("    {{}} // row {row}\n");
        // Block comments as row labels.
        let block_labels = |row: usize| format!("    /* {:04X} */ {{}}\n", row * 4);
        for (name, comment) in
            [("//", &line_comments as &dyn Fn(usize) -> String), ("/* */", &block_labels)]
        {
            let src = table(20, comment);
            // Alone, no row reaches the window...
            let runs = literal_runs(src.as_bytes(), Comments::Break);
            assert!(runs.iter().all(|run| run.bytes.len() <= 4), "{name}");
            // ... but the table is flagged, from the line of its first row.
            assert_eq!(index.find_literal(src.as_bytes()), Some(2), "{name}");
            // Fifteen bytes are still not a window.
            assert_eq!(index.find_literal(table(15, comment).as_bytes()), None, "{name}");
        }
    }

    #[test]
    fn comment_hex_is_not_injected_into_a_run() {
        let rom = fake_rom();
        let index = RomIndex::new(&rom);
        // 16 ROM bytes with a commented-out `0xFF` between every pair; reading the comment would
        // insert bytes that are not in the ROM and hide the run.
        let table = rom[1000..1016]
            .chunks(2)
            .map(|pair| format!("{} // was 0xFF, 0xFE\n", hex_list(pair)))
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(index.find_literal(table.as_bytes()), Some(1));
    }

    #[test]
    fn an_index_without_windows_is_empty() {
        assert!(RomIndex::new(&[]).is_empty());
        assert!(RomIndex::new(&fake_rom()[..10]).is_empty());
        assert!(RomIndex::new(&vec![0_u8; 4096]).is_empty());
        assert!(RomIndex::new(&[0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0]).is_empty());
        assert!(!RomIndex::new(&fake_rom()).is_empty());
    }

    #[test]
    fn finds_rom_bytes_pasted_as_byte_strings() {
        let rom = fake_rom();
        let index = RomIndex::new(&rom);
        let src = |n: usize| format!("static T: &[u8] = {};\n", byte_string(&rom[3000..3000 + n]));
        assert_eq!(index.find_literal(src(18).as_bytes()), Some(1));
        assert_eq!(index.find_literal(src(15).as_bytes()), None);
    }

    #[test]
    fn literal_scan_ignores_our_own_and_low_entropy_bytes() {
        let rom = fake_rom();
        let index = RomIndex::new(&rom);
        // Foreign bytes (a synthetic fixture) do not match the ROM.
        let own: Vec<u8> = (0..24_u32).map(|i| (i * 11 + 5) as u8).collect();
        assert_eq!(index.find_literal(hex_list(&own).as_bytes()), None);
        // Low-entropy runs are ignored even when they are in the ROM.
        let zeros = vec![0_u8; 64];
        let zero_index = RomIndex::new(&zeros);
        assert_eq!(zero_index.find_literal(hex_list(&zeros).as_bytes()), None);
    }
}
