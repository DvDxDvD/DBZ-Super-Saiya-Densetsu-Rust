//! `export-public <commit> <out-dir>`: assembles the tree of the public repository from a commit of
//! this one, and scans it before anyone publishes it.
//!
//! The tree is the commit's tracked files (nothing from the working directory) minus what is for
//! the maintainers only (`tools/public/exclude.txt`: one path or folder per line), with the public
//! front of the project laid over it: the files of `tools/public/` (README, LICENSE, NOTICE,
//! `.gitignore`, the Cargo config) take their public places, and the developer reference that is
//! this repository's own `README.md` becomes `docs/DEVELOPER.md`. Wording that should not be
//! public is fixed in the sources, not here.
//!
//! The scan of the result fails the export (exit 1) on any finding:
//!
//! * the `publish-check` rules: forbidden file types and places, and any run of 16 bytes of the
//!   cached ROM in a file or written as byte literals (the scan is off, and the export fails,
//!   without the ROM unless `--allow-no-rom`);
//! * the rules of `tools/public/scan-rules.txt`, a file the export does not ship: `secret` rules
//!   (keys, tokens, addresses, hosts, places on a person's disk) over every file, and `process`
//!   rules (words of how the project was made rather than of what it is) over every file but
//!   `third_party/`. `tools/public/scan-allow.txt` lists the findings that were looked at and are
//!   fine (`kind|path prefix|text that the line has`).
//!
//! `docs/media/` holds the pictures of the README: images are allowed there and in `assets/`.

use crate::publish_check::{self, Finding, RomIndex};
use anyhow::Context;
use regex::Regex;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// Where the public front of the project is kept in a commit.
const TEMPLATES: &str = "tools/public/";

/// `(file of tools/public/, its place in the public tree)`.
const OVERLAYS: &[(&str, &str)] = &[
    ("README.md", "README.md"),
    ("LICENSE", "LICENSE"),
    ("NOTICE.md", "NOTICE.md"),
    ("LICENSE-ART.md", "LICENSE-ART.md"),
    ("gitignore", ".gitignore"),
    ("cargo-config.toml", ".cargo/config.toml"),
];

/// Files that are placed elsewhere in the public tree: `(path in the commit, public path)`.
const MOVED: &[(&str, &str)] = &[("README.md", "docs/DEVELOPER.md")];

/// Paths that are never exported whatever `exclude.txt` says: the working directories, the files
/// the public front replaces, and the templates themselves.
const ALWAYS_EXCLUDED: &[&str] =
    &[".gitignore", ".cargo/config.toml", ".work/", "target/", TEMPLATES];

/// The paths of `exclude.txt`: a file, or a folder with its trailing `/`, one per line.
fn parse_excluded(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The public path of the tracked file `path`, or `None` when it stays private.
fn destination(path: &str, excluded: &[String]) -> Option<String> {
    if let Some(&(_, to)) = MOVED.iter().find(|&&(from, _)| from == path) {
        return Some(to.to_owned());
    }
    let hit = |rule: &str| if rule.ends_with('/') { path.starts_with(rule) } else { path == rule };
    let private = ALWAYS_EXCLUDED.iter().any(|&rule| hit(rule)) || excluded.iter().any(|r| hit(r));
    (!private).then(|| path.to_owned())
}

/// Runs git in `repo`.
fn git(repo: &Path, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    anyhow::ensure!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(out.stdout)
}

/// The files of `commit`: `(path, object id)` of every regular file.
fn tree(repo: &Path, commit: &str) -> anyhow::Result<Vec<(String, String)>> {
    let listing = git(repo, &["ls-tree", "-r", "-z", commit])?;
    let mut files = Vec::new();
    for entry in listing.split(|&b| b == 0).filter(|entry| !entry.is_empty()) {
        // `<mode> <type> <id>\t<path>`
        let text = String::from_utf8_lossy(entry);
        let (meta, path) = text.split_once('\t').context("a tree entry")?;
        let mut parts = meta.split(' ');
        let (mode, kind, id) = (parts.next(), parts.next(), parts.next());
        if kind == Some("blob") && matches!(mode, Some("100644" | "100755")) {
            files.push((path.to_owned(), id.context("an object id")?.to_owned()));
        }
    }
    Ok(files)
}

/// The contents of the objects `ids`, in order, through one `git cat-file --batch`.
fn blobs(repo: &Path, ids: &[&str]) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("running git cat-file")?;
    let mut stdin = child.stdin.take().context("git's stdin")?;
    let owned: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    let writer = std::thread::spawn(move || {
        for id in owned {
            if writeln!(stdin, "{id}").is_err() {
                break;
            }
        }
    });
    let mut stdout = std::io::BufReader::new(child.stdout.take().context("git's stdout")?);
    let mut out = Vec::with_capacity(ids.len());
    for _ in ids {
        let mut header = String::new();
        stdout.read_line(&mut header)?;
        let size: usize = header
            .split_whitespace()
            .nth(2)
            .and_then(|size| size.parse().ok())
            .with_context(|| format!("git cat-file said {header:?}"))?;
        let mut data = vec![0; size];
        stdout.read_exact(&mut data)?;
        let mut newline = [0_u8; 1];
        stdout.read_exact(&mut newline)?;
        out.push(data);
    }
    writer.join().map_err(|_| anyhow::anyhow!("the git writer panicked"))?;
    child.wait()?;
    Ok(out)
}

/// One rule of `scan-rules.txt`.
struct Rule {
    kind: Kind,
    regex: Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Secret,
    Process,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Process => "process",
        }
    }
}

/// The rules of `text`: one per line, `secret <regex>` or `process <regex>`, `#` starts a comment.
/// A `process` rule matches without regard to case.
fn parse_rules(text: &str) -> anyhow::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (kind, pattern) = line
            .split_once(char::is_whitespace)
            .with_context(|| format!("scan-rules line {}: `kind regex`", number + 1))?;
        let (kind, pattern) = match kind {
            "secret" => (Kind::Secret, pattern.trim().to_owned()),
            "process" => (Kind::Process, format!("(?i){}", pattern.trim())),
            other => anyhow::bail!("scan-rules line {}: the kind `{other}`", number + 1),
        };
        let regex = Regex::new(&pattern)
            .with_context(|| format!("scan-rules line {}: the pattern", number + 1))?;
        rules.push(Rule { kind, regex });
    }
    Ok(rules)
}

/// A finding that was looked at and is fine: any line of `kind` in a file under `path` that has
/// `text` in it.
struct Allowed {
    kind: String,
    path: String,
    text: String,
}

fn parse_allowed(text: &str) -> Vec<Allowed> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            Some(Allowed {
                kind: parts.next()?.to_owned(),
                path: parts.next()?.to_owned(),
                text: parts.next()?.to_owned(),
            })
        })
        .collect()
}

/// Whether `bytes` are text to scan: UTF-8 with no NUL.
fn as_text(bytes: &[u8]) -> Option<&str> {
    if bytes.contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

/// The findings of the rules in the text of `path`.
fn scan_text(path: &str, text: &str, rules: &[Rule], allowed: &[Allowed]) -> Vec<Finding> {
    // (the notes about our patches of the vendored code are ours; the vendored code is not)
    let third_party = path.starts_with("third_party/") && !path.ends_with("PATCHES.md");
    let mut findings = Vec::new();
    for (number, line) in text.lines().enumerate() {
        for rule in rules {
            if rule.kind == Kind::Process && third_party {
                continue;
            }
            let Some(found) = rule.regex.find(line) else { continue };
            let fine = allowed.iter().any(|a| {
                a.kind == rule.kind.name() && path.starts_with(&a.path) && line.contains(&a.text)
            });
            if fine {
                continue;
            }
            let mut shown: String = line.trim().chars().take(140).collect();
            if shown.is_empty() {
                found.as_str().clone_into(&mut shown);
            }
            findings.push(Finding {
                path: format!("{path}:{}", number + 1),
                reason: format!("{} `{}`: {shown}", rule.kind.name(), found.as_str()),
            });
        }
    }
    findings
}

/// Every file under `dir`, as `/`-separated paths relative to it, sorted.
fn walk(dir: &Path) -> anyhow::Result<Vec<String>> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<String>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(root, &path, out)?;
            } else {
                let relative = path.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
                out.push(relative);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Whether a path of the public tree may be an image: `assets/` and `docs/media/`.
fn is_media(path: &str) -> bool {
    path.starts_with("docs/media/")
}

/// The scan of the tree under `out`: the `publish-check` rules and the rules of the commit.
fn scan(
    out: &Path,
    rom: Option<&RomIndex>,
    rules: &[Rule],
    allowed: &[Allowed],
) -> anyhow::Result<(Vec<Finding>, usize)> {
    let paths = walk(out)?;
    // The file types and places are the publish check's (pictures: assets/ and docs/media/ only)
    let mut findings = publish_check::check_paths(&paths);
    for path in &paths {
        let data = std::fs::read(out.join(path)).with_context(|| format!("reading {path}"))?;
        if publish_check::is_vetted(path, &data) {
            continue;
        }
        if let Some(index) = rom
            && !is_media(path)
        {
            if let Some(offset) = index.find_run(&data) {
                findings.push(Finding {
                    path: path.clone(),
                    reason: format!("contains ROM bytes at offset {offset}"),
                });
            } else if publish_check::is_literal_scanned(path)
                && let Some(line) = index.find_literal(&data)
            {
                findings.push(Finding {
                    path: path.clone(),
                    reason: format!("contains ROM bytes as literals (run at line {line})"),
                });
            }
        }
        if let Some(text) = as_text(&data) {
            findings.extend(scan_text(path, text, rules, allowed));
        }
    }
    Ok((findings, paths.len()))
}

/// What the command line asked.
struct Args {
    commit: String,
    out: PathBuf,
    allow_no_rom: bool,
}

fn parse(args: &[&str]) -> Option<Args> {
    let mut positional = Vec::new();
    let mut allow_no_rom = false;
    for &arg in args {
        match arg {
            "--allow-no-rom" => allow_no_rom = true,
            other if !other.starts_with("--") => positional.push(other),
            _ => return None,
        }
    }
    let [commit, out] = positional.as_slice() else { return None };
    Some(Args { commit: (*commit).to_owned(), out: PathBuf::from(out), allow_no_rom })
}

/// Exports `commit` of the repository at `repo` to `out` and scans it. `rom` is the cached ROM, if
/// there is one.
pub fn run(
    repo: &Path,
    rom: Option<&[u8]>,
    args: &[&str],
    report: &mut dyn Write,
) -> anyhow::Result<ExitCode> {
    let Some(args) = parse(args) else { return Ok(ExitCode::from(crate::EXIT_USAGE)) };
    let commit = String::from_utf8(git(
        repo,
        &["rev-parse", "--verify", &format!("{}^{{commit}}", args.commit)],
    )?)?
    .trim()
    .to_owned();
    if args.out.exists() && std::fs::read_dir(&args.out)?.next().is_some() {
        anyhow::bail!("{} is not empty: export into a new folder", args.out.display());
    }
    let rom_index = rom.map(RomIndex::new).filter(|index| !index.is_empty());
    if rom_index.is_none() && !args.allow_no_rom {
        anyhow::bail!(
            "the ROM scan is off (no cached ROM): launch the game once with --rom, or pass --allow-no-rom"
        );
    }
    let files = tree(repo, &commit)?;
    let template_path = |name: &str| format!("{TEMPLATES}{name}");
    let excluded = match files.iter().find(|(path, _)| *path == template_path("exclude.txt")) {
        Some((_, id)) => parse_excluded(&String::from_utf8(blobs(repo, &[id])?.remove(0))?),
        None => anyhow::bail!("{TEMPLATES}exclude.txt is missing in {commit}"),
    };
    let wanted: Vec<(&str, &str)> = files
        .iter()
        .filter_map(|(path, id)| destination(path, &excluded).map(|_| (path.as_str(), id.as_str())))
        .collect();
    // The files of the tree, then the public front
    let ids: Vec<&str> = wanted.iter().map(|&(_, id)| id).collect();
    let contents = blobs(repo, &ids)?;
    std::fs::create_dir_all(&args.out)?;
    let write = |public: &str, data: &[u8]| -> anyhow::Result<()> {
        let target = args.out.join(public);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, data).with_context(|| format!("writing {}", target.display()))
    };
    for (&(path, _), data) in wanted.iter().zip(&contents) {
        write(&destination(path, &excluded).context("a public path")?, data)?;
    }
    let find = |name: &str| files.iter().find(|(path, _)| *path == template_path(name));
    for &(from, to) in OVERLAYS {
        let (_, id) =
            find(from).with_context(|| format!("{TEMPLATES}{from} is missing in {commit}"))?;
        write(to, &blobs(repo, &[id])?.remove(0))?;
    }
    write("docs/media/.gitkeep", b"")?;
    // The rules of the scan
    let text_of = |name: &str| -> anyhow::Result<String> {
        let (_, id) = find(name).with_context(|| format!("{TEMPLATES}{name} is missing"))?;
        Ok(String::from_utf8(blobs(repo, &[id])?.remove(0))?)
    };
    let rules = parse_rules(&text_of("scan-rules.txt")?)?;
    let allowed = parse_allowed(&text_of("scan-allow.txt").unwrap_or_default());
    // The scan
    let (findings, count) = scan(&args.out, rom_index.as_ref(), &rules, &allowed)?;
    for finding in &findings {
        writeln!(report, "{}: {}", finding.path, finding.reason)?;
    }
    let bytes: u64 = walk(&args.out)?
        .iter()
        .filter_map(|path| std::fs::metadata(args.out.join(path)).ok())
        .map(|meta| meta.len())
        .sum();
    writeln!(
        report,
        "export-public: {count} files, {:.1} MB, from {} into {} (ROM scan: {}, {} rules)",
        bytes as f64 / 1_048_576.0,
        &commit[..12],
        args.out.display(),
        if rom_index.is_some() { "on" } else { "off" },
        rules.len()
    )?;
    if findings.is_empty() {
        writeln!(report, "export-public: scan clean")?;
        Ok(ExitCode::SUCCESS)
    } else {
        writeln!(
            report,
            "export-public: {} findings, the export is not publishable",
            findings.len()
        )?;
        Ok(ExitCode::from(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_files_stay_out_and_the_readme_becomes_the_developer_guide() {
        let excluded = parse_excluded("# the maintainers' files\nNOTES.md\n\ninternal/\n");
        for private in [
            "NOTES.md",
            "internal/plans/x.md",
            ".gitignore",
            ".cargo/config.toml",
            "tools/public/README.md",
            ".work/a.txt",
            "target/debug/x",
        ] {
            assert_eq!(destination(private, &excluded), None, "{private}");
        }
        let place = |path: &str| destination(path, &excluded);
        assert_eq!(place("README.md").as_deref(), Some("docs/DEVELOPER.md"));
        assert_eq!(
            place("crates/ssd-rom/src/lib.rs").as_deref(),
            Some("crates/ssd-rom/src/lib.rs")
        );
        assert_eq!(place("docs/re/secrets.md").as_deref(), Some("docs/re/secrets.md"));
        assert_eq!(
            place("internals.md").as_deref(),
            Some("internals.md"),
            "a folder rule is a folder"
        );
    }

    #[test]
    fn rules_are_parsed_by_kind_and_the_process_ones_ignore_case() {
        let rules = parse_rules("# a comment\n\nsecret ghp_[a-z]{4}\nprocess some word\n").unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules[0].regex.is_match("a ghp_abcd b") && !rules[0].regex.is_match("GHP_ABCD"));
        assert!(rules[1].regex.is_match("Some WORD here"));
        assert!(parse_rules("nonsense x").is_err());
        assert!(parse_rules("secret (").is_err());
        assert!(parse_rules("secret").is_err());
    }

    #[test]
    fn a_finding_names_the_line_and_the_allow_list_forgives_it() {
        let rules = parse_rules("secret tok_[0-9]+\nprocess hush\n").unwrap();
        let text = "fine\nhas tok_123 in it\nHUSH here\n";
        let findings = scan_text("crates/a.rs", text, &rules, &[]);
        let at: Vec<&str> = findings.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(at, ["crates/a.rs:2", "crates/a.rs:3"]);
        let allowed = parse_allowed("# note\nsecret|crates/|tok_123\nprocess|other/|hush\n");
        let findings = scan_text("crates/a.rs", text, &rules, &allowed);
        assert_eq!(findings.len(), 1, "only the process word is left: {}", findings[0].reason);
        // A process word is not looked for in third-party code, a secret is
        let findings = scan_text("third_party/x/lib.rs", text, &rules, &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].reason.starts_with("secret"));
    }

    #[test]
    fn binary_files_are_not_read_as_text() {
        assert!(as_text(b"plain").is_some());
        assert!(as_text(&[0x41, 0, 0x42]).is_none());
        assert!(as_text(&[0xFF, 0xFE, 0x41]).is_none());
    }

    #[test]
    fn pictures_are_allowed_in_docs_media_only() {
        let dir = tempfile::tempdir().unwrap();
        for (path, data) in [
            ("docs/media/cmp-a.png", &b"x"[..]),
            ("docs/shot.png", b"x"),
            ("assets/fonts/a.png", b"x"),
            ("docs/media/a.exe", b"x"),
            ("src/lib.rs", b"fn main() {}"),
        ] {
            let target = dir.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, data).unwrap();
        }
        let (findings, count) = scan(dir.path(), None, &[], &[]).unwrap();
        let mut flagged: Vec<&str> = findings.iter().map(|f| f.path.as_str()).collect();
        flagged.sort_unstable();
        assert_eq!(count, 5);
        assert_eq!(flagged, ["docs/media/a.exe", "docs/shot.png"]);
    }

    #[test]
    fn the_command_line_takes_a_commit_and_a_folder() {
        assert!(parse(&["main", "out"]).is_some());
        assert!(parse(&["main", "out", "--allow-no-rom"]).is_some_and(|a| a.allow_no_rom));
        assert!(parse(&["main"]).is_none());
        assert!(parse(&["main", "out", "--force"]).is_none());
    }
}
