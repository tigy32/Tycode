/// The result of a "find in file" operation that finds the closest match to a
/// given vec of lines in a test.
///
/// When searching text, AIs (even opus!) tend to fail to make tool calls for
/// file edits correctly (either by not understanding the tool, or getting
/// spacing or something wrong in the search block). When this happens we return
/// a helpful error "did you mean?" style error message to help guide them
/// towards a valid tool call. Perhaps we could just accept the file if its
/// "close enough".
///
/// We previously gave models back the incorrect text they gave us as the error
/// message. Someone on reddit gave a good example of why this was dumb agent
/// behavior:
/// "DO NOT SAY APPLE. DEFINITELY DO NOT SAY APPLE. WHATEVER YOU SAY IN RESPONSE
/// TO MY NEXT REQUEST DO NOT SAY APPLE. Ok, name a fruit."
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub matched_lines: Vec<String>,
    pub start_index: usize,
    /// Similarity score (0.0 = no match, 1.0 = perfect match)
    pub similarity: f64,
}

impl MatchResult {
    /// Returns correction feedback if the match is not exact
    /// Returns None for perfect matches (similarity = 1.0)
    pub fn get_correction_feedback(&self) -> Option<String> {
        if self.similarity >= 1.0 {
            return None;
        }

        let mut feedback = String::new();
        feedback.push_str(&format!(
            "Found closest match with {:.1}% similarity at line {}\n\n",
            self.similarity * 100.0,
            self.start_index + 1
        ));
        feedback.push_str("Closest match:\n");

        for line in &self.matched_lines {
            feedback.push_str(&format!("{line}\n"));
        }

        Some(feedback)
    }
}

/// Find the closest matching section in source lines for the given search lines
pub fn find_closest_match(source: Vec<String>, search: Vec<String>) -> Option<MatchResult> {
    if search.is_empty() || source.is_empty() {
        return None;
    }

    if search.len() > source.len() {
        return None;
    }

    let mut best_match: Option<(usize, f64, Vec<String>)> = None;

    // Slide window through source
    for i in 0..=source.len() - search.len() {
        let window: Vec<String> = source[i..i + search.len()].to_vec();
        let similarity = calculate_similarity(&window, &search);

        match &best_match {
            None => best_match = Some((i, similarity, window)),
            Some((_, best_sim, _)) if similarity > *best_sim => {
                best_match = Some((i, similarity, window));
            }
            _ => {}
        }
    }

    best_match.map(|(start_index, similarity, matched_lines)| MatchResult {
        matched_lines,
        start_index,
        similarity,
    })
}

fn calculate_similarity(window: &[String], search: &[String]) -> f64 {
    let mut total_similarity = 0.0;

    for (window_line, search_line) in window.iter().zip(search.iter()) {
        let line_similarity = calculate_line_similarity(window_line, search_line);
        total_similarity += line_similarity;
    }

    total_similarity / search.len() as f64
}

fn calculate_line_similarity(s1: &str, s2: &str) -> f64 {
    if s1 == s2 {
        return 1.0;
    }

    let distance = levenshtein_distance(s1, s2);
    let max_len = s1.len().max(s2.len());

    if max_len == 0 {
        return 1.0;
    }

    1.0 - (distance as f64 / max_len as f64)
}

fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.len();
    let len2 = s2.len();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut prev_row: Vec<usize> = (0..=len2).collect();
    let mut curr_row = vec![0; len2 + 1];

    for (i, c1) in s1.chars().enumerate() {
        curr_row[0] = i + 1;

        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[len2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let source = vec![
            "line 1".to_string(),
            "line 2".to_string(),
            "line 3".to_string(),
        ];
        let search = vec!["line 2".to_string()];

        let result = find_closest_match(source, search).unwrap();
        assert_eq!(result.start_index, 1);
        assert_eq!(result.similarity, 1.0);
        assert_eq!(result.matched_lines, vec!["line 2".to_string()]);
    }

    #[test]
    fn test_fuzzy_match() {
        let source = vec![
            "if ft.is_dir() {".to_string(),
            " return true;".to_string(),
            "}".to_string(),
        ];
        let search = vec![
            "if ft.is_dir() {".to_string(),
            " return true".to_string(), // Missing semicolon
        ];

        let result = find_closest_match(source, search).unwrap();
        assert_eq!(result.start_index, 0);
        assert!(result.similarity > 0.9);
        assert_eq!(result.matched_lines[0], "if ft.is_dir() {");
    }

    #[test]
    fn test_multiline_match() {
        let source = vec![
            "None => return false,".to_string(),
            "Some(ft) => ft,".to_string(),
            "};".to_string(),
            "if ft.is_dir() {".to_string(),
            "return true;".to_string(),
            "}".to_string(),
        ];
        let search = vec![
            "None => return false,".to_string(),
            "Some(ft) => ft,".to_string(),
            "};".to_string(),
            "if ft.is_dir() {".to_string(),
            "return true".to_string(),
        ];

        let result = find_closest_match(source.clone(), search).unwrap();
        assert_eq!(result.start_index, 0);
        assert!(result.similarity > 0.95);
    }

    #[test]
    fn test_performance_full_file() {
        let full_file = r#"/!
Defines a builder for haystacks.
A "haystack" represents something we want to search. It encapsulates the logic
for whether a haystack ought to be searched or not, separate from the standard
ignore rules and other filtering logic.
Effectively, a haystack wraps a directory entry and adds some light application
level logic around it.
/
use std::path::Path;
/// A builder for constructing things to search over.
#[derive(Clone, Debug)]
pub(crate) struct HaystackBuilder {
 strip_dot_prefix: bool,
}
impl HaystackBuilder {
 /// Return a new haystack builder with a default configuration.
 pub(crate) fn new() -> HaystackBuilder {
 HaystackBuilder { strip_dot_prefix: false }
 }
 /// Create a new haystack from a possibly missing directory entry.
 ///
 /// If the directory entry isn't present, then the corresponding error is
 /// logged if messages have been configured. Otherwise, if the directory
 /// entry is deemed searchable, then it is returned as a haystack.
 pub(crate) fn build_from_result(
 &self,
 result: Result<ignore::DirEntry, ignore::Error>,
 ) -> Option<Haystack> {
 match result {
 Ok(dent) => self.build(dent),
 Err(err) => {
 err_message!("{err}");
 None
 }
 }
 }
 /// Create a new haystack using this builder's configuration.
 ///
 /// If a directory entry could not be created or should otherwise not be
 /// searched, then this returns None after emitting any relevant log
 /// messages.
 fn build(&self, dent: ignore::DirEntry) -> Option<Haystack> {
 let hay = Haystack { dent, strip_dot_prefix: self.strip_dot_prefix };
 if let Some(err) = hay.dent.error() {
 ignore_message!("{err}");
 }
 // If this entry was explicitly provided by an end user, then we always
 // want to search it.
 if hay.is_explicit() {
 return Some(hay);
 }
 // At this point, we only want to search something if it's explicitly a
 // file. This omits symlinks. (If ripgrep was configured to follow
 // symlinks, then they have already been followed by the directory
 // traversal.)
 if hay.is_file() {
 return Some(hay);
 }
 // We got nothing. Emit a debug message, but only if this isn't a
 // directory. Otherwise, emitting messages for directories is just
 // noisy.
 if !hay.is_dir() {
 log::debug!(
 "ignoring {}: failed to pass haystack filter: \
 file type: {:?}, metadata: {:?}",
 hay.dent.path().display(),
 hay.dent.file_type(),
 hay.dent.metadata()
 );
 }
 None
 }
 /// When enabled, if the haystack's file path starts with ./ then it is
 /// stripped.
 ///
 /// This is useful when implicitly searching the current working directory.
 pub(crate) fn strip_dot_prefix(
 &mut self,
 yes: bool,
 ) -> &mut HaystackBuilder {
 self.strip_dot_prefix = yes;
 self
 }
}
/// A haystack is a thing we want to search.
///
/// Generally, a haystack is either a file or stdin.
#[derive(Clone, Debug)]
pub(crate) struct Haystack {
 dent: ignore::DirEntry,
 strip_dot_prefix: bool,
}
impl Haystack {
 /// Return the file path corresponding to this haystack.
 ///
 /// If this haystack corresponds to stdin, then a special <stdin> path
 /// is returned instead.
 pub(crate) fn path(&self) -> &Path {
 if self.strip_dot_prefix && self.dent.path().starts_with("./") {
 self.dent.path().strip_prefix("./").unwrap()
 } else {
 self.dent.path()
 }
 }
 /// Returns true if and only if this entry corresponds to stdin.
 pub(crate) fn is_stdin(&self) -> bool {
 self.dent.is_stdin()
 }
 /// Returns true if and only if this entry corresponds to a haystack to
 /// search that was explicitly supplied by an end user.
 ///
 /// Generally, this corresponds to either stdin or an explicit file path
 /// argument. e.g., in rg foo some-file ./some-dir/, some-file is
 /// an explicit haystack, but, e.g., ./some-dir/some-other-file is not.
 ///
 /// However, note that ripgrep does not see through shell globbing. e.g.,
 /// in rg foo ./some-dir/*, ./some-dir/some-other-file will be treated
 /// as an explicit haystack.
 pub(crate) fn is_explicit(&self) -> bool {
 // stdin is obvious. When an entry has a depth of 0, that means it
 // was explicitly provided to our directory iterator, which means it
 // was in turn explicitly provided by the end user. The !is_dir check
 // means that we want to search files even if their symlinks, again,
 // because they were explicitly provided. (And we never want to try
 // to search a directory.)
 self.is_stdin() || (self.dent.depth() == 0 && !self.is_dir())
 }
 /// Returns true if and only if this haystack points to a directory after
 /// following symbolic links.
 fn is_dir(&self) -> bool {
 let ft = match self.dent.file_type() {
 None => return false,
 Some(ft) => ft,
 };
 if ft.is_dir() {
 return true;
 }
 // If this is a symlink, then we want to follow it to determine
 // whether it's a directory or not.
 self.dent.path_is_symlink() && self.dent.path().is_dir()
 }
 /// Returns true if and only if this haystack points to a file.
 fn is_file(&self) -> bool {
 self.dent.file_type().map_or(false, |ft| ft.is_file())
 }
}"#;

        let source: Vec<String> = full_file.lines().map(|s| s.to_string()).collect();
        let search = vec![
            " None => return false,".to_string(),
            " Some(ft) => ft,".to_string(),
            " };".to_string(),
            " if ft.is_dir() {".to_string(),
            " return true;".to_string(),
        ];

        use std::time::Instant;
        let start = Instant::now();
        let result = find_closest_match(source.clone(), search).unwrap();
        let duration = start.elapsed();

        println!("Search took: {:?} for {} lines", duration, source.len());
        assert_eq!(result.start_index, 131);
        assert!(result.similarity > 0.95);
        assert_eq!(result.matched_lines[4], " return true;");
    }

    #[test]
    fn test_correction_feedback() {
        let source = vec![
            "let ft = match self.dent.file_type() {".to_string(),
            " None => return false,".to_string(),
            " Some(ft) => ft,".to_string(),
            "};".to_string(),
            "if ft.is_dir() {".to_string(),
            " return true;".to_string(),
            "}".to_string(),
        ];

        // Search with missing semicolon and wrong indentation
        let search = vec![
            "None => return false,".to_string(),
            " Some(ft) => ft,".to_string(),
            " };".to_string(),
            " if ft.is_dir() {".to_string(),
            " return true".to_string(),
        ];

        let result = find_closest_match(source, search).unwrap();
        let feedback = result.get_correction_feedback();

        // Should return Some feedback for imperfect match
        assert!(feedback.is_some());

        let feedback_text = feedback.unwrap();
        assert!(feedback_text.contains("None => return false,"));
        println!("Correction feedback:\n{feedback_text}");
    }

    #[test]
    fn test_no_feedback_for_perfect_match() {
        let source = vec![
            "line 1".to_string(),
            "line 2".to_string(),
            "line 3".to_string(),
        ];
        let search = vec!["line 2".to_string()];

        let result = find_closest_match(source, search).unwrap();
        let feedback = result.get_correction_feedback();

        // Should return None for perfect match
        assert!(feedback.is_none());
    }
}
