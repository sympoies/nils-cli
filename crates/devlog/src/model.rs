//! Devlog domain model: locating the log, parsing month files, and the
//! structural rules `docs/devlog/README.md` states in prose.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Directory names that hold a development log, in detection order.
///
/// Two conventions exist across the organization: most repositories use
/// `docs/devlog`, while a repository with a source/render split keeps its
/// authored copy under `docs/source/devlog`. Detection covers both so a
/// caller never has to say which one this repository uses.
pub const DEVLOG_DIRS: [&str; 2] = ["docs/devlog", "docs/source/devlog"];

/// A located development log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Devlog {
    repo_root: PathBuf,
    dir: PathBuf,
}

impl Devlog {
    /// Locate the log under `repo_root`, trying each known directory in order.
    pub fn locate(repo_root: &Path) -> Result<Self, DevlogError> {
        for candidate in DEVLOG_DIRS {
            let dir = repo_root.join(candidate);
            if dir.is_dir() {
                return Ok(Self {
                    repo_root: repo_root.to_path_buf(),
                    dir,
                });
            }
        }
        Err(DevlogError::NotFound {
            repo_root: repo_root.to_path_buf(),
        })
    }

    /// Use an explicit directory instead of detection.
    pub fn at(repo_root: &Path, dir: &Path) -> Result<Self, DevlogError> {
        if !dir.is_dir() {
            return Err(DevlogError::NotADirectory {
                path: dir.to_path_buf(),
            });
        }
        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// The log directory relative to the repository root, using forward
    /// slashes so output is identical on every platform.
    pub fn relative_dir(&self) -> String {
        self.relative(&self.dir)
    }

    /// `path` relative to the repository root, forward-slashed.
    ///
    /// Output is identical on every platform so reported paths can be compared
    /// and pasted regardless of where the log was checked out.
    ///
    /// A path outside the repository root is returned unchanged apart from the
    /// slash normalization. `--dir` pointing at another checkout's log is the
    /// ordinary way to check a log this repository does not own, so that is a
    /// supported route and what it prints has to stay pastable: joining the
    /// components of an absolute path with `/` would render its root as a
    /// second separator, and POSIX leaves a leading `//` implementation-defined.
    pub fn relative(&self, path: &Path) -> String {
        let path = path.strip_prefix(&self.repo_root).unwrap_or(path);
        let mut rendered = String::new();
        for component in path.components() {
            match component {
                // The root and a Windows prefix are the separator, so they are
                // written as-is and never preceded by one.
                Component::RootDir => rendered.push('/'),
                // Unreachable on every platform this workspace builds for: CI
                // runs Linux and macOS only. It is here so the match says what
                // a prefix is rather than letting the arm below invent a
                // separator in front of one, not because the Windows rendering
                // has been verified.
                Component::Prefix(prefix) => {
                    rendered.push_str(&prefix.as_os_str().to_string_lossy());
                }
                component => {
                    if !rendered.is_empty() && !rendered.ends_with('/') {
                        rendered.push('/');
                    }
                    rendered.push_str(&component.as_os_str().to_string_lossy());
                }
            }
        }
        rendered
    }

    pub fn index_path(&self) -> PathBuf {
        self.dir.join("README.md")
    }

    pub fn month_path(&self, month: Month) -> PathBuf {
        self.dir.join(format!("{month}.md"))
    }

    /// Every tracked month file, oldest first.
    ///
    /// A file whose stem is not a valid `YYYY-MM` is reported rather than
    /// skipped: a silently ignored mis-named entry is the failure mode this
    /// crate exists to prevent.
    pub fn months(&self) -> Result<MonthScan, DevlogError> {
        let mut months = Vec::new();
        let mut unexpected = Vec::new();

        let entries = std::fs::read_dir(&self.dir).map_err(|source| DevlogError::Io {
            path: self.dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| DevlogError::Io {
                path: self.dir.clone(),
                source,
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                unexpected.push(path);
                continue;
            };
            if name == "README.md" {
                continue;
            }
            match name.strip_suffix(".md").map(str::parse::<Month>) {
                Some(Ok(month)) => months.push(month),
                _ => unexpected.push(path),
            }
        }

        months.sort();
        unexpected.sort();
        Ok(MonthScan { months, unexpected })
    }
}

/// The result of scanning the log directory for month files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthScan {
    /// Valid `YYYY-MM` month files, oldest first.
    pub months: Vec<Month>,
    /// Files that do not match the `YYYY-MM.md` convention.
    pub unexpected: Vec<PathBuf>,
}

/// A `YYYY-MM` month identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month {
    year: i16,
    month: i8,
}

impl Month {
    pub fn new(year: i16, month: i8) -> Result<Self, DevlogError> {
        if !(1..=12).contains(&month) {
            return Err(DevlogError::InvalidMonth {
                value: format!("{year:04}-{month:02}"),
            });
        }
        Ok(Self { year, month })
    }

    pub fn year(self) -> i16 {
        self.year
    }

    pub fn month(self) -> i8 {
        self.month
    }

    /// The `# Development log - YYYY-MM` heading a month file opens with.
    pub fn heading(self) -> String {
        format!("# Development log - {self}")
    }
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl std::str::FromStr for Month {
    type Err = DevlogError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || DevlogError::InvalidMonth {
            value: value.to_string(),
        };
        let (year, month) = value.split_once('-').ok_or_else(invalid)?;
        if year.len() != 4 || month.len() != 2 {
            return Err(invalid());
        }
        if !year.bytes().all(|b| b.is_ascii_digit()) || !month.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let year: i16 = year.parse().map_err(|_| invalid())?;
        let month: i8 = month.parse().map_err(|_| invalid())?;
        Self::new(year, month)
    }
}

/// A `YYYY-MM-DD` entry date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryDate {
    month: Month,
    day: i8,
}

impl EntryDate {
    pub fn new(month: Month, day: i8) -> Result<Self, DevlogError> {
        let max = days_in_month(month);
        if !(1..=max).contains(&day) {
            return Err(DevlogError::InvalidDate {
                value: format!("{month}-{day:02}"),
            });
        }
        Ok(Self { month, day })
    }

    /// Today's date in the system time zone.
    pub fn today() -> Result<Self, DevlogError> {
        let now = jiff::Zoned::now().date();
        let month = Month::new(now.year(), now.month())?;
        Self::new(month, now.day())
    }

    pub fn month(self) -> Month {
        self.month
    }

    pub fn day(self) -> i8 {
        self.day
    }
}

impl fmt::Display for EntryDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:02}", self.month, self.day)
    }
}

impl std::str::FromStr for EntryDate {
    type Err = DevlogError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || DevlogError::InvalidDate {
            value: value.to_string(),
        };
        let (month, day) = value.rsplit_once('-').ok_or_else(invalid)?;
        if day.len() != 2 || !day.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let month: Month = month.parse().map_err(|_| invalid())?;
        let day: i8 = day.parse().map_err(|_| invalid())?;
        Self::new(month, day)
    }
}

fn days_in_month(month: Month) -> i8 {
    match month.month() {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(month.year()) => 29,
        2 => 28,
        // `Month::new` rejects every other value, so this arm is unreachable
        // for a constructed `Month`.
        _ => 0,
    }
}

fn is_leap_year(year: i16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Errors this crate reports to its callers.
#[derive(Debug)]
pub enum DevlogError {
    NotFound {
        repo_root: PathBuf,
    },
    NotADirectory {
        path: PathBuf,
    },
    InvalidMonth {
        value: String,
    },
    InvalidDate {
        value: String,
    },
    MissingMonthFile {
        path: PathBuf,
    },
    MissingHeading {
        path: PathBuf,
        expected: String,
    },
    ConflictMarkers {
        path: PathBuf,
        line: usize,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    NotAGitWorkTree,
}

/// Line prefixes git writes into a file it could not merge.
///
/// Month files conflict whenever two branches add an entry in the same month,
/// because every entry is inserted at the same position. That makes an
/// unresolved conflict the most likely damage a month file will ever carry, so
/// both reading and writing have to recognize it.
///
/// The separator `=======` is deliberately absent: on its own line it is also a
/// Markdown setext heading underline, and these files are prose. The opening,
/// ancestor (`diff3` style), and closing markers are unambiguous, and a
/// conflict always writes an opening and a closing one, so leaving the
/// separator out costs no detection.
const CONFLICT_MARKERS: [&str; 3] = ["<<<<<<<", "|||||||", ">>>>>>>"];

/// The 1-based line number of the first conflict marker in `contents`, ignoring
/// fenced code blocks.
///
/// An entry that documents merge-conflict handling quotes these markers inside
/// a fence, and the entry describing this very behavior is the obvious example.
/// Treating a quoted marker as a real one would make the month permanently
/// unwritable by `new` until someone edited the prose, which is a worse failure
/// than the one this function exists to catch. Git writes markers at column
/// zero in the file body, never inside a fence it did not already break, so
/// skipping fenced regions costs no real detection.
pub fn first_conflict_marker(contents: &str) -> Option<usize> {
    let mut fences = FenceScanner::default();

    for (index, line) in contents.lines().enumerate() {
        if !fences.is_structural(line) {
            continue;
        }
        if CONFLICT_MARKERS
            .iter()
            .any(|marker| line.starts_with(marker))
        {
            return Some(index + 1);
        }
    }

    None
}

/// Tracks, line by line, whether a month file is inside a fenced code block.
///
/// Everything this crate parses out of a month file — conflict markers, entry
/// headings, section headings — is structure only outside a fence. Inside one
/// it is an example, and the entry documenting this very format is the obvious
/// case. `check` reading a quoted `## 2026-04-17 - Title` as a real entry would
/// report problems nobody can fix without editing the prose, and `fix` would
/// then write a backfilled section into the middle of the code block.
///
/// One scanner, shared by every reader, is what keeps them agreeing about what
/// an entry is.
#[derive(Debug, Default)]
pub struct FenceScanner<'a> {
    open: Option<&'a str>,
}

impl<'a> FenceScanner<'a> {
    /// Advance over `line` and report whether it carries structure.
    ///
    /// A line that opens or closes a fence, and every line between them, does
    /// not.
    pub fn is_structural(&mut self, line: &'a str) -> bool {
        if let Some(open) = self.open {
            // A fence closes on a run of the same character at least as long as
            // the one that opened it, per CommonMark.
            if fence_delimiter(line).is_some_and(|close| close.starts_with(open)) {
                self.open = None;
            }
            return false;
        }
        if let Some(open) = fence_delimiter(line) {
            self.open = Some(open);
            return false;
        }
        true
    }
}

/// The leading run of backticks or tildes when `line` opens or closes a fence.
///
/// Public because `fix` rewrites lines in place and must not rewrite one that
/// a fence has turned into an example — an entry documenting this very format
/// is the obvious case, and the conflict scan above already excludes it for
/// the same reason.
pub fn fence_delimiter(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for character in ['`', '~'] {
        let run = trimmed.split(|c| c != character).next().unwrap_or_default();
        if run.len() >= 3 {
            return Some(run);
        }
    }
    None
}

impl DevlogError {
    /// The stable error code emitted in the JSON envelope.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "devlog-not-found",
            Self::NotADirectory { .. } => "not-a-directory",
            Self::InvalidMonth { .. } => "invalid-month",
            Self::InvalidDate { .. } => "invalid-date",
            Self::MissingMonthFile { .. } => "missing-month-file",
            Self::MissingHeading { .. } => "missing-heading",
            Self::ConflictMarkers { .. } => "conflict-markers",
            Self::Io { .. } => "io-error",
            Self::NotAGitWorkTree => "not-a-git-work-tree",
        }
    }
}

impl fmt::Display for DevlogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { repo_root } => write!(
                f,
                "no development log under {}: expected one of {}",
                repo_root.display(),
                DEVLOG_DIRS.join(" or ")
            ),
            Self::NotADirectory { path } => {
                write!(f, "not a directory: {}", path.display())
            }
            Self::InvalidMonth { value } => {
                write!(f, "expected a YYYY-MM month, got '{value}'")
            }
            Self::InvalidDate { value } => {
                write!(f, "expected a YYYY-MM-DD date, got '{value}'")
            }
            Self::MissingMonthFile { path } => {
                write!(f, "no devlog file for month: {}", path.display())
            }
            Self::ConflictMarkers { path, line } => write!(
                f,
                "{} has an unresolved merge conflict at line {line}; resolve it before writing",
                path.display()
            ),
            Self::MissingHeading { path, expected } => write!(
                f,
                "{} does not open with its expected heading '{}'",
                path.display(),
                expected
            ),
            Self::Io { path, source } => {
                write!(f, "{}: {source}", path.display())
            }
            Self::NotAGitWorkTree => {
                write!(f, "must run inside a git work tree")
            }
        }
    }
}

impl std::error::Error for DevlogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
