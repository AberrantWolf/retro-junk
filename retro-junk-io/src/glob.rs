//! Loose path patterns, for the places a person describes a group of files.
//!
//! Reviewing a thousand unaccounted files one at a time is not a workflow, and
//! the useful groupings are shaped like paths: "anything ending `.txt`",
//! "everything under a folder named `rvz`". So the review surfaces let you
//! describe a group instead of selecting one, and this is the one place that
//! decides what a description covers.
//!
//! It has to be one place. The same pattern is used to filter what the Inbox
//! shows, to count what a bulk button is about to act on, and — once saved as
//! a durable rule — to decide what the adoption sweep files in the first
//! place. If those three disagreed even slightly, a button labelled "dismiss
//! 412" would dismiss a different 412 than the list showed.
//!
//! The syntax is the familiar shell-glob one, with one deliberate difference:
//!
//! * `*` matches any run of characters **including `/`**, so `*.txt` finds
//!   `psx/notes.txt` and does not need to be written `*/*.txt`. Directory
//!   boundaries are not special here, because the thing being matched is one
//!   whole relative path rather than one path component.
//! * `?` matches exactly one character.
//! * `[abc]`, `[a-z]`, `[!abc]` match one character from a set, or outside it.
//! * A pattern with none of those characters is read as "contains this", so
//!   typing `zelda` finds every path with `zelda` anywhere in it. Requiring
//!   `*zelda*` would only ever surprise people.
//!
//! Matching ignores case for ASCII letters, since `*.TXT` and `*.txt` are
//! plainly meant to be the same request. Letters outside ASCII match exactly:
//! folding them can change a string's length, which would break the position
//! arithmetic below, and the payoff — a case-insensitive match on a Japanese
//! title someone typed as a filter — is not worth an unsound matcher.

/// One parsed pattern, ready to test many paths against.
///
/// Parsing once matters: the Inbox re-tests every open row on every keystroke,
/// and the adoption sweep tests every file against every saved rule.
///
/// The default is the empty pattern, which matches everything — so a filter
/// that has not been typed into yet hides nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pattern {
    tokens: Vec<Token>,
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// One literal character, already lowercased if it is ASCII.
    Literal(char),
    /// `*` — any run of characters, `/` included.
    Any,
    /// `?` — exactly one character.
    One,
    /// `[…]` — one character from a set, or (negated) one outside it.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

impl Token {
    /// Whether this token accepts one character. `Any` is never asked — it
    /// consumes a variable run and is handled by the matcher's backtracking.
    fn accepts(&self, value: char) -> bool {
        match self {
            Self::Literal(expected) => *expected == value,
            Self::One => true,
            Self::Class { negated, ranges } => {
                let inside = ranges
                    .iter()
                    .any(|(low, high)| value >= *low && value <= *high);
                inside != *negated
            }
            Self::Any => false,
        }
    }
}

impl Pattern {
    /// Parse a pattern. Any input is a valid pattern — an unterminated `[`
    /// matches a literal bracket rather than failing, because this parses what
    /// a person typed into a filter box mid-keystroke.
    #[must_use]
    pub fn new(pattern: &str) -> Self {
        let source = pattern.to_owned();
        let has_wildcard = pattern.contains(['*', '?', '[']);
        let mut tokens = Vec::new();
        // A bare word means "contains", so it is matched as if it were
        // surrounded by wildcards.
        if !has_wildcard && !pattern.is_empty() {
            tokens.push(Token::Any);
        }
        let characters: Vec<char> = pattern.chars().collect();
        let mut index = 0;
        while index < characters.len() {
            match characters[index] {
                '*' => {
                    // Runs of `*` are the same request as one `*`, and
                    // collapsing them keeps the matcher's backtracking cheap.
                    if tokens.last() != Some(&Token::Any) {
                        tokens.push(Token::Any);
                    }
                    index += 1;
                }
                '?' => {
                    tokens.push(Token::One);
                    index += 1;
                }
                '[' => {
                    if let Some((token, next)) = parse_class(&characters, index) {
                        tokens.push(token);
                        index = next;
                    } else {
                        tokens.push(Token::Literal('['));
                        index += 1;
                    }
                }
                literal => {
                    tokens.push(Token::Literal(literal.to_ascii_lowercase()));
                    index += 1;
                }
            }
        }
        if !has_wildcard && !pattern.is_empty() {
            tokens.push(Token::Any);
        }
        Self { tokens, source }
    }

    /// The pattern as it was typed, for showing back to the user.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// True when the pattern selects nothing in particular — an empty filter
    /// box. Callers use this to mean "no filter" rather than "matches
    /// nothing".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Whether this pattern describes `text`.
    ///
    /// An empty pattern matches everything, so a caller can hold one
    /// unconditionally and let an empty filter box mean "show all".
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        if self.tokens.is_empty() {
            return true;
        }
        let text: Vec<char> = text
            .chars()
            .map(|value| value.to_ascii_lowercase())
            .collect();
        // Standard wildcard matching: walk both strings together, and whenever
        // a `*` is passed, remember where it was. On a mismatch, rewind to the
        // most recent `*` and let it swallow one more character. Every `*` only
        // ever grows, so this terminates.
        let mut token = 0;
        let mut position = 0;
        let mut resume: Option<(usize, usize)> = None;
        while position < text.len() {
            match self.tokens.get(token) {
                Some(Token::Any) => {
                    resume = Some((token, position));
                    token += 1;
                }
                Some(current) if current.accepts(text[position]) => {
                    token += 1;
                    position += 1;
                }
                _ => match resume {
                    Some((star, from)) => {
                        token = star + 1;
                        position = from + 1;
                        resume = Some((star, from + 1));
                    }
                    None => return false,
                },
            }
        }
        // Trailing wildcards are allowed to match nothing.
        self.tokens[token..]
            .iter()
            .all(|token| matches!(token, Token::Any))
    }
}

/// Parse a `[...]` set starting at `open`, returning the token and the index
/// just past the closing bracket. `None` means the bracket never closed.
fn parse_class(characters: &[char], open: usize) -> Option<(Token, usize)> {
    let mut index = open + 1;
    let negated = characters.get(index).is_some_and(|value| *value == '!');
    if negated {
        index += 1;
    }
    let mut ranges = Vec::new();
    // A `]` in the first position is a literal `]`, as in shell globs.
    let mut first = true;
    while index < characters.len() {
        let value = characters[index];
        if value == ']' && !first {
            // An empty set would match nothing and is almost certainly a typo;
            // treating it as a literal bracket is the kinder reading.
            if ranges.is_empty() {
                return None;
            }
            return Some((Token::Class { negated, ranges }, index + 1));
        }
        first = false;
        let low = value.to_ascii_lowercase();
        // `a-z`, but a `-` at the end of the set is a literal `-`.
        if characters.get(index + 1) == Some(&'-')
            && let Some(high) = characters.get(index + 2)
            && *high != ']'
        {
            ranges.push((low, high.to_ascii_lowercase()));
            index += 3;
        } else {
            ranges.push((low, low));
            index += 1;
        }
    }
    None
}

/// Whether `pattern` describes `text`. For one-off tests; hold a [`Pattern`]
/// when testing many strings against the same pattern.
#[must_use]
pub fn matches(pattern: &str, text: &str) -> bool {
    Pattern::new(pattern).matches(text)
}

#[cfg(test)]
#[path = "tests/glob_tests.rs"]
mod tests;
