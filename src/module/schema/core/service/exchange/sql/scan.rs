#[derive(Debug, Clone, PartialEq)]
pub(super) enum Token {
    /// A bare word: a keyword or an unquoted identifier. PostgreSQL folds an
    /// unquoted name to lower case, so that is how it is stored.
    Word(String),
    Quoted(String),
    Text(String),
    Number(String),
    Punct(char),
}

impl Token {
    pub(super) fn is_word(&self, word: &str) -> bool {
        matches!(self, Token::Word(found) if found == word)
    }

    pub(super) fn is_punct(&self, punct: char) -> bool {
        matches!(self, Token::Punct(found) if *found == punct)
    }

    /// Either spelling of a name, with the quoted one keeping its case.
    pub(super) fn name(&self) -> Option<&str> {
        match self {
            Token::Word(word) | Token::Quoted(word) => Some(word),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Statement {
    pub(super) tokens: Vec<Token>,
    /// The original text, kept so a statement nobody can read can be named
    /// back to the person who wrote it.
    pub(super) source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LayoutHint {
    pub(super) table: String,
    pub(super) x: f64,
    pub(super) y: f64,
}

pub(super) fn scan(sql: &str) -> (Vec<Statement>, Vec<LayoutHint>) {
    Scanner::new(sql).run()
}

struct Scanner {
    chars: Vec<char>,
    at: usize,
    statements: Vec<Statement>,
    hints: Vec<LayoutHint>,
    tokens: Vec<Token>,
    started_at: usize,
    token_at: usize,
}

impl Scanner {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            at: 0,
            statements: Vec::new(),
            hints: Vec::new(),
            tokens: Vec::new(),
            started_at: 0,
            token_at: 0,
        }
    }

    fn run(mut self) -> (Vec<Statement>, Vec<LayoutHint>) {
        while self.at < self.chars.len() {
            let character = self.chars[self.at];

            // Where this token starts, so the statement's own text can be
            // sliced back out of the source for a warning.
            self.token_at = self.at;

            match character {
                ';' => {
                    self.at += 1;
                    self.finish();
                }
                '-' if self.looks_at("--") => self.line_comment(),
                '/' if self.looks_at("/*") => self.block_comment(),
                '\'' => self.quoted('\'', Token::Text),
                '"' => self.quoted('"', Token::Quoted),
                '$' if self.dollar_tag().is_some() => self.dollar_quoted(),
                _ if character.is_whitespace() => self.at += 1,
                _ if character.is_ascii_digit() => self.number(),
                _ if is_name_start(character) => self.word(),
                _ => {
                    self.push(Token::Punct(character));
                    self.at += 1;
                }
            }
        }

        self.finish();

        (self.statements, self.hints)
    }

    fn push(&mut self, token: Token) {
        if self.tokens.is_empty() {
            self.started_at = self.token_at;
        }

        self.tokens.push(token);
    }

    fn finish(&mut self) {
        if self.tokens.is_empty() {
            self.started_at = self.at;
            return;
        }

        let source = self.slice(self.started_at, self.at).trim().to_owned();

        self.statements.push(Statement {
            tokens: std::mem::take(&mut self.tokens),
            source,
        });
        self.started_at = self.at;
    }

    fn slice(&self, from: usize, to: usize) -> String {
        self.chars[from.min(self.chars.len())..to.min(self.chars.len())]
            .iter()
            .collect()
    }

    fn looks_at(&self, prefix: &str) -> bool {
        prefix
            .chars()
            .enumerate()
            .all(|(offset, expected)| self.chars.get(self.at + offset) == Some(&expected))
    }

    fn line_comment(&mut self) {
        let start = self.at;

        while self.at < self.chars.len() && self.chars[self.at] != '\n' {
            self.at += 1;
        }

        if let Some(hint) = layout_hint(&self.slice(start, self.at)) {
            self.hints.push(hint);
        }
    }

    /// PostgreSQL nests block comments, so a closing marker only ends the
    /// comment it belongs to.
    fn block_comment(&mut self) {
        let mut depth = 0;

        while self.at < self.chars.len() {
            if self.looks_at("/*") {
                depth += 1;
                self.at += 2;
            } else if self.looks_at("*/") {
                depth -= 1;
                self.at += 2;

                if depth == 0 {
                    return;
                }
            } else {
                self.at += 1;
            }
        }
    }

    /// A doubled delimiter inside the run is the delimiter itself, which is
    /// how both `'it''s'` and `"od""d"` are written.
    fn quoted(&mut self, delimiter: char, wrap: fn(String) -> Token) {
        self.at += 1;

        let mut value = String::new();

        while self.at < self.chars.len() {
            let character = self.chars[self.at];

            if character == delimiter {
                if self.chars.get(self.at + 1) == Some(&delimiter) {
                    value.push(delimiter);
                    self.at += 2;
                    continue;
                }

                self.at += 1;
                break;
            }

            value.push(character);
            self.at += 1;
        }

        self.push(wrap(value));
    }

    fn dollar_tag(&self) -> Option<String> {
        let mut at = self.at + 1;
        let mut tag = String::new();

        while let Some(character) = self.chars.get(at) {
            match character {
                '$' => return Some(tag),
                _ if is_name_char(*character) => {
                    tag.push(*character);
                    at += 1;
                }
                _ => return None,
            }
        }

        None
    }

    fn dollar_quoted(&mut self) {
        let Some(tag) = self.dollar_tag() else { return };
        let delimiter = format!("${tag}$");

        self.at += delimiter.chars().count();

        let start = self.at;

        while self.at < self.chars.len() && !self.looks_at(&delimiter) {
            self.at += 1;
        }

        let value = self.slice(start, self.at);

        self.at = (self.at + delimiter.chars().count()).min(self.chars.len());
        self.push(Token::Text(value));
    }

    fn number(&mut self) {
        let start = self.at;

        while self
            .chars
            .get(self.at)
            .is_some_and(|character| character.is_ascii_digit() || *character == '.')
        {
            self.at += 1;
        }

        let value = self.slice(start, self.at);
        self.push(Token::Number(value));
    }

    fn word(&mut self) {
        let start = self.at;

        while self.chars.get(self.at).is_some_and(|c| is_name_char(*c)) {
            self.at += 1;
        }

        let value = self.slice(start, self.at).to_lowercase();
        self.push(Token::Word(value));
    }
}

fn is_name_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

fn is_name_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// `-- @table "users" 120 340`, the block the generator writes. Anything else
/// on a comment line is just a comment.
fn layout_hint(line: &str) -> Option<LayoutHint> {
    let rest = line.trim_start_matches('-').trim().strip_prefix("@table")?;
    let rest = rest.trim();

    let (table, rest) = match rest.strip_prefix('"') {
        Some(quoted) => {
            let (name, rest) = split_quoted(quoted)?;
            (name, rest)
        }
        None => {
            let end = rest.find(char::is_whitespace)?;
            (rest[..end].to_owned(), &rest[end..])
        }
    };

    let mut numbers = rest.split_whitespace();
    let x = numbers.next()?.parse().ok()?;
    let y = numbers.next()?.parse().ok()?;

    Some(LayoutHint { table, x, y })
}

fn split_quoted(input: &str) -> Option<(String, &str)> {
    let mut name = String::new();
    let mut characters = input.char_indices();

    while let Some((at, character)) = characters.next() {
        if character != '"' {
            name.push(character);
            continue;
        }

        if input[at + 1..].starts_with('"') {
            name.push('"');
            characters.next();
            continue;
        }

        return Some((name, &input[at + 1..]));
    }

    None
}
