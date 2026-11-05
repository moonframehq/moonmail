use moonmail_core::{MailPredicate as P, TextField, TextTerm};

#[derive(Debug, thiserror::Error)]
#[error("Invalid search: {0}")]
pub struct QueryError(pub &'static str);

#[derive(Debug, PartialEq)]
enum Token {
    Term(String, Option<usize>),
    And,
    Or,
    Not,
    Left,
    Right,
}

/// Uppercase NOT > explicit/implicit AND > OR. Parentheses override precedence.
/// Quotes retain literal phrases (including operator words); backslash escapes
/// quote/backslash. Field names are lowercase. Date boundaries are UTC midnight.
pub fn parse_query(input: &str) -> Result<P, QueryError> {
    if input.len() > 4096 {
        return Err(QueryError("at most 4096 bytes"));
    }
    let tokens = lex(input)?;
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let predicate = parser.or(0)?;
    if parser.position != parser.tokens.len() {
        return Err(QueryError("unexpected closing parenthesis or operator"));
    }
    Ok(predicate)
}

fn lex(input: &str) -> Result<Vec<Token>, QueryError> {
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if tokens.len() >= 128 {
            return Err(QueryError("too many query tokens"));
        }
        if c == '(' {
            tokens.push(Token::Left);
            continue;
        }
        if c == ')' {
            tokens.push(Token::Right);
            continue;
        }
        let mut value = String::new();
        let mut quoted = false;
        let mut had_quote = false;
        let mut colon = None;
        let mut current = Some(c);
        while let Some(c) = current {
            match c {
                '"' => {
                    quoted = !quoted;
                    had_quote = true;
                },
                '\\' => {
                    let escaped = chars.next().ok_or(QueryError("unfinished escape"))?;
                    if !matches!(escaped, '"' | '\\') {
                        return Err(QueryError("escape only quotes or backslashes"));
                    }
                    value.push(escaped);
                },
                ':' if !quoted && colon.is_none() => {
                    colon = Some(value.len());
                    value.push(c);
                },
                _ => value.push(c),
            }
            if !quoted
                && chars
                    .peek()
                    .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | ')'))
            {
                break;
            }
            current = chars.next();
        }
        if quoted {
            return Err(QueryError("unclosed quote"));
        }
        if value.is_empty() {
            return Err(QueryError("empty search term"));
        }
        tokens.push(if !had_quote {
            match value.as_str() {
                "AND" => Token::And,
                "OR" => Token::Or,
                "NOT" => Token::Not,
                _ => Token::Term(value, colon),
            }
        } else {
            Token::Term(value, colon)
        });
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}
impl Parser {
    fn or(&mut self, depth: usize) -> Result<P, QueryError> {
        let mut children = vec![self.and(depth)?];
        while self.tokens.get(self.position) == Some(&Token::Or) {
            self.position += 1;
            children.push(self.and(depth)?);
        }
        Ok(if children.len() == 1 {
            children.remove(0)
        } else {
            P::Any(children)
        })
    }
    fn and(&mut self, depth: usize) -> Result<P, QueryError> {
        let mut children = vec![self.atom(depth)?];
        loop {
            match self.tokens.get(self.position) {
                Some(Token::And) => self.position += 1,
                Some(Token::Term(..) | Token::Not | Token::Left) => {},
                _ => break,
            }
            children.push(self.atom(depth)?);
        }
        Ok(if children.len() == 1 {
            children.remove(0)
        } else {
            P::All(children)
        })
    }
    fn atom(&mut self, depth: usize) -> Result<P, QueryError> {
        if depth > 16 {
            return Err(QueryError("nesting exceeds 16 levels"));
        }
        let token = self
            .tokens
            .get(self.position)
            .ok_or(QueryError("expected a search term"))?;
        self.position += 1;
        match token {
            Token::Not => Ok(P::Not(Box::new(self.atom(depth + 1)?))),
            Token::Left => {
                let result = self.or(depth + 1)?;
                if self.tokens.get(self.position) != Some(&Token::Right) {
                    return Err(QueryError("missing closing parenthesis"));
                }
                self.position += 1;
                Ok(result)
            },
            Token::Term(text, colon) => term(text, *colon),
            _ => Err(QueryError("expected a term, NOT or parenthesis")),
        }
    }
}

fn term(text: &str, colon: Option<usize>) -> Result<P, QueryError> {
    let (field, value) = colon.map_or(("", text), |i| (&text[..i], &text[i + 1..]));
    if value.trim().is_empty() {
        return Err(QueryError("field value is empty"));
    }
    let field = match field {
        "" => TextField::Any,
        "subject" => TextField::Subject,
        "body" => TextField::Body,
        "from" => TextField::From,
        "to" => TextField::To,
        "cc" => TextField::Cc,
        "account" => return Ok(P::Account(value.into())),
        "in" => return Ok(P::Mailbox(value.into())),
        "sender-address" => return Ok(P::SenderAddress(value.into())),
        "recipient-address" => return Ok(P::RecipientAddress(value.into())),
        "keyword" => {
            return moonmail_core::canonical_keyword(value)
                .map(P::Keyword)
                .ok_or(QueryError("invalid IMAP keyword atom"));
        },
        "tag" => return Ok(P::Tag(value.into())),
        "is" => {
            return match value {
                "unread" => Ok(P::Unread),
                "read" => Ok(P::Not(Box::new(P::Unread))),
                "starred" => Ok(P::Starred),
                _ => Err(QueryError("is: accepts unread, read or starred")),
            };
        },
        "after" => return Ok(P::After(date(value)?)),
        "before" => return Ok(P::Before(date(value)?)),
        _ => {
            return Err(QueryError(
                "unknown field; quote literal text containing a colon",
            ));
        },
    };
    Ok(P::Text(TextTerm {
        field,
        value: value.into(),
    }))
}

fn date(value: &str) -> Result<i64, QueryError> {
    if !value.is_ascii() || value.len() != 10 || &value[4..5] != "-" || &value[7..8] != "-" {
        return Err(QueryError("dates use YYYY-MM-DD"));
    }
    let invalid = || QueryError("invalid calendar date");
    let year = value[..4].parse().map_err(|_| invalid())?;
    let month = value[5..7]
        .parse::<u8>()
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())?;
    let day = value[8..].parse().map_err(|_| invalid())?;
    Ok(time::Date::from_calendar_date(year, month, day)
        .map_err(|_| invalid())?
        .midnight()
        .assume_utc()
        .unix_timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_fields_quotes_and_utc_dates_are_explicit() -> Result<(), QueryError> {
        assert_eq!(
            parse_query("subject:\"design review\" OR from:clara NOT is:read")?,
            P::Any(vec![
                P::Text(TextTerm {
                    field: TextField::Subject,
                    value: "design review".into()
                }),
                P::All(vec![
                    P::Text(TextTerm {
                        field: TextField::From,
                        value: "clara".into()
                    }),
                    P::Not(Box::new(P::Not(Box::new(P::Unread))))
                ])
            ])
        );
        assert_eq!(
            parse_query("after:2024-02-29 before:2024-03-01")?,
            P::All(vec![P::After(1_709_164_800), P::Before(1_709_251_200)])
        );
        assert_eq!(
            parse_query("\"OR\"")?,
            P::Text(TextTerm {
                field: TextField::Any,
                value: "OR".into()
            })
        );
        assert_eq!(
            parse_query("\"https://example.org\"")?,
            P::Text(TextTerm {
                field: TextField::Any,
                value: "https://example.org".into()
            })
        );
        assert!(matches!(
            parse_query("(from:alice OR cc:bob) is:unread")?,
            P::All(_)
        ));
        Ok(())
    }
    #[test]
    fn malformed_and_excessive_queries_fail_instead_of_broadening_results() {
        for input in [
            "",
            "foo OR",
            "AND foo",
            "(foo",
            "foo)",
            "\"foo",
            "subject:",
            "is:missing",
            "before:2023-02-29",
            "before:😃-01-01",
            "bogus:bar",
            "()",
            "foo AND OR bar",
        ] {
            assert!(parse_query(input).is_err(), "{input}");
        }
        assert!(parse_query(&format!("{}foo{}", "(".repeat(18), ")".repeat(18))).is_err());
        assert!(parse_query(&"x ".repeat(200)).is_err());
    }
}
