use super::scan::Token;

pub(super) struct Cursor<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, at: 0 }
    }

    pub(super) fn done(&self) -> bool {
        self.at >= self.tokens.len()
    }

    pub(super) fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    pub(super) fn next(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.at);

        if token.is_some() {
            self.at += 1;
        }

        token
    }

    pub(super) fn at_word(&self, word: &str) -> bool {
        self.peek().is_some_and(|token| token.is_word(word))
    }

    pub(super) fn at_punct(&self, punct: char) -> bool {
        self.peek().is_some_and(|token| token.is_punct(punct))
    }

    pub(super) fn eat_word(&mut self, word: &str) -> bool {
        let found = self.at_word(word);

        if found {
            self.at += 1;
        }

        found
    }

    /// Several keywords in a row, taken only when all of them are there, so
    /// `PRIMARY` on its own does not look like `PRIMARY KEY`.
    pub(super) fn eat_words(&mut self, words: &[&str]) -> bool {
        let found = words.iter().enumerate().all(|(offset, word)| {
            self.tokens
                .get(self.at + offset)
                .is_some_and(|token| token.is_word(word))
        });

        if found {
            self.at += words.len();
        }

        found
    }

    pub(super) fn eat_punct(&mut self, punct: char) -> bool {
        let found = self.at_punct(punct);

        if found {
            self.at += 1;
        }

        found
    }

    /// A name, with the schema kept on the front when it carries one, so
    /// `public.users` and `users` stay apart on the canvas.
    pub(super) fn qualified_name(&mut self) -> Option<String> {
        let mut name = self.next()?.name()?.to_owned();

        while self.at_punct('.') {
            self.at += 1;

            let part = self.next()?.name()?.to_owned();
            name = format!("{name}.{part}");
        }

        Some(name)
    }

    /// The names inside `(a, b, c)`.
    pub(super) fn name_list(&mut self) -> Option<Vec<String>> {
        if !self.eat_punct('(') {
            return None;
        }

        let mut names = Vec::new();

        loop {
            names.push(self.next()?.name()?.to_owned());

            if self.eat_punct(',') {
                continue;
            }

            return self.eat_punct(')').then_some(names);
        }
    }

    /// Everything up to the next comma or closing paren at this depth, which
    /// is how a default expression or a check body is skipped over whole.
    pub(super) fn take_until_separator(&mut self, stop: &[&str]) -> Vec<Token> {
        let mut depth = 0;
        let mut taken = Vec::new();

        while let Some(token) = self.peek() {
            match token {
                Token::Punct('(') => depth += 1,
                Token::Punct(')') if depth == 0 => break,
                Token::Punct(')') => depth -= 1,
                Token::Punct(',') if depth == 0 => break,
                Token::Word(word) if depth == 0 && stop.contains(&word.as_str()) => break,
                _ => {}
            }

            taken.push(token.clone());
            self.at += 1;
        }

        taken
    }

    pub(super) fn skip_balanced(&mut self) {
        if !self.eat_punct('(') {
            return;
        }

        let mut depth = 1;

        while let Some(token) = self.next() {
            if token.is_punct('(') {
                depth += 1;
            } else if token.is_punct(')') {
                depth -= 1;

                if depth == 0 {
                    return;
                }
            }
        }
    }
}

/// Tokens written back out as SQL. A default expression goes to the database
/// as text, so it only has to mean the same thing, not look identical.
pub(super) fn render(tokens: &[Token]) -> String {
    let mut out = String::new();

    for token in tokens {
        let piece = match token {
            Token::Word(word) => word.clone(),
            Token::Quoted(name) => format!("\"{}\"", name.replace('"', "\"\"")),
            Token::Text(text) => format!("'{}'", text.replace('\'', "''")),
            Token::Number(number) => number.clone(),
            Token::Punct(punct) => punct.to_string(),
        };

        let joined =
            !out.is_empty() && !matches!(piece.as_str(), ")" | "," | "(") && !out.ends_with('(');

        if joined {
            out.push(' ');
        }

        out.push_str(&piece);
    }

    out
}
