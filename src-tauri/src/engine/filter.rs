//! FreeFileSync-style include/exclude filter.
//!
//! Pattern rules (case-insensitive, `*` = any characters, `?` = one character):
//! - A pattern without `\` matches the item *name* at any level: `*.tmp`, `Thumbs.db`, `node_modules`.
//! - A pattern with `\` matches the *path* relative to the base folder: `\Projects\*.bak`.
//!   A leading `\` is optional.
//! - A trailing `\` limits the pattern to folders: `\Cache\` excludes that folder and everything inside.
//! - An excluded folder is not scanned at all.
//! - Include patterns decide which files are kept (default `*` = all). Folders are always scanned.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            include: vec!["*".into()],
            exclude: vec![
                r"\System Volume Information\".into(),
                r"\$Recycle.Bin\".into(),
                r"\RECYCLER\".into(),
                r"\RECYCLED\".into(),
                "Thumbs.db".into(),
                "desktop.ini".into(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
struct Pattern {
    glob: Vec<char>,
    /// Matches the relative path (with a leading `\`) instead of the name.
    by_path: bool,
    /// Matches folders only.
    folders_only: bool,
}

impl Pattern {
    fn parse(raw: &str) -> Option<Self> {
        let mut p = raw.trim().replace('/', "\\").to_lowercase();
        if p.is_empty() || p == "\\" {
            return None;
        }
        let folders_only = p.ends_with('\\');
        while p.ends_with('\\') {
            p.pop();
        }
        let by_path = p.contains('\\');
        if by_path && !p.starts_with('\\') {
            p.insert(0, '\\');
        }
        Some(Self {
            glob: p.chars().collect(),
            by_path,
            folders_only,
        })
    }

    /// `rel_path` uses `\` separators and has no leading `\`.
    fn matches(&self, rel_path_lower: &str, is_folder: bool) -> bool {
        if self.folders_only && !is_folder {
            return false;
        }
        if self.by_path {
            let target: Vec<char> = std::iter::once('\\').chain(rel_path_lower.chars()).collect();
            glob_match(&self.glob, &target)
        } else {
            let name = rel_path_lower.rsplit('\\').next().unwrap_or(rel_path_lower);
            let target: Vec<char> = name.chars().collect();
            glob_match(&self.glob, &target)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Filter {
    include: Vec<Pattern>,
    exclude: Vec<Pattern>,
}

impl Default for Filter {
    fn default() -> Self {
        Self::new(&FilterConfig::default())
    }
}

impl Filter {
    pub fn new(cfg: &FilterConfig) -> Self {
        let mut include: Vec<Pattern> = cfg.include.iter().filter_map(|p| Pattern::parse(p)).collect();
        if include.is_empty() {
            include = Pattern::parse("*").into_iter().collect();
        }
        Self {
            include,
            exclude: cfg.exclude.iter().filter_map(|p| Pattern::parse(p)).collect(),
        }
    }

    /// A filter that keeps everything.
    pub fn none() -> Self {
        Self::new(&FilterConfig {
            include: vec!["*".into()],
            exclude: vec![],
        })
    }

    /// Should this folder be skipped entirely (not scanned, not shown)?
    pub fn excludes_folder(&self, rel_path: &str) -> bool {
        let lower = rel_path.to_lowercase();
        self.exclude.iter().any(|p| p.matches(&lower, true))
    }

    /// Should this file be part of the comparison?
    /// Assumes its parent folders were already checked with [`Filter::excludes_folder`].
    pub fn keeps_file(&self, rel_path: &str) -> bool {
        let lower = rel_path.to_lowercase();
        if self.exclude.iter().any(|p| p.matches(&lower, false)) {
            return false;
        }
        if self.include.iter().any(|p| p.matches(&lower, false)) {
            return true;
        }
        // A folder-level include pattern (e.g. `\Docs\`) keeps every file inside that folder.
        let mut ancestor = lower.as_str();
        while let Some(pos) = ancestor.rfind('\\') {
            ancestor = &ancestor[..pos];
            if self.include.iter().any(|p| p.matches(ancestor, true)) {
                return true;
            }
        }
        false
    }
}

/// Wildcard match where `*` matches any run of characters (including `\`) and `?` matches one.
fn glob_match(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_t = 0usize;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            star_t = t;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(include: &[&str], exclude: &[&str]) -> Filter {
        Filter::new(&FilterConfig {
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn glob_basics() {
        let g = |p: &str, t: &str| glob_match(&p.chars().collect::<Vec<_>>(), &t.chars().collect::<Vec<_>>());
        assert!(g("*.tmp", "a.tmp"));
        assert!(g("*", ""));
        assert!(g("a?c", "abc"));
        assert!(!g("a?c", "ac"));
        assert!(g("*a*b*", "xxaxxbxx"));
        assert!(!g("*.tmp", "a.tmpx"));
    }

    #[test]
    fn name_patterns_match_at_any_level() {
        let f = filter(&["*"], &["*.tmp", "node_modules"]);
        assert!(!f.keeps_file("a.tmp"));
        assert!(!f.keeps_file(r"deep\inside\B.TMP"));
        assert!(f.keeps_file(r"deep\a.txt"));
        assert!(f.excludes_folder(r"src\node_modules"));
        assert!(!f.excludes_folder(r"src\node_modules_backup"));
    }

    #[test]
    fn dot_git_does_not_hide_gitignore() {
        // The old Relax wrongly skipped .gitignore when ".git" was excluded.
        let f = filter(&["*"], &[".git"]);
        assert!(f.excludes_folder(r"project\.git"));
        assert!(f.keeps_file(r"project\.gitignore"));
        assert!(!f.excludes_folder(r"project\.github"));
    }

    #[test]
    fn path_patterns_are_anchored_to_base() {
        let f = filter(&["*"], &[r"\Cache\", r"Projects\*.bak"]);
        assert!(f.excludes_folder("Cache"));
        assert!(f.excludes_folder("cache"));
        assert!(!f.excludes_folder(r"sub\Cache"));
        assert!(!f.keeps_file(r"Projects\x.bak"));
        assert!(f.keeps_file(r"Other\x.bak"));
    }

    #[test]
    fn folder_only_patterns_ignore_files() {
        let f = filter(&["*"], &[r"build\"]);
        assert!(f.excludes_folder(r"a\build"));
        assert!(f.keeps_file(r"a\build"));
    }

    #[test]
    fn include_patterns() {
        let f = filter(&["*.docx", r"\Photos\"], &[]);
        assert!(f.keeps_file(r"a\report.DOCX"));
        assert!(!f.keeps_file(r"a\report.pdf"));
        assert!(f.keeps_file(r"Photos\2026\img.jpg"));
        assert!(!f.keeps_file(r"Other\img.jpg"));
    }

    #[test]
    fn empty_include_means_everything() {
        let f = filter(&[], &[]);
        assert!(f.keeps_file("anything.bin"));
    }

    #[test]
    fn default_excludes_system_folders() {
        let f = Filter::default();
        assert!(f.excludes_folder("System Volume Information"));
        assert!(f.excludes_folder("$RECYCLE.BIN"));
        assert!(!f.keeps_file(r"a\Thumbs.db"));
        assert!(f.keeps_file(r"a\photo.jpg"));
    }
}
