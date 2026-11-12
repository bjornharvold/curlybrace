//! One `db...` expression of a script read as a collection call: its receiver
//! (`db.<name>`, `db.<a>.<b>` for a dotted name, `db.getCollection("<name>")`, each optionally
//! after `db.getSiblingDB("<name>")`), its arguments and its chained calls. Every step works on
//! paired tokens, so it costs no more than the tokens it reads at its own nesting level.
use super::shell_literal::{is_blank, parse_literal, parse_string};
use super::shell_queries::{METHODS, ShellCall};
use super::shell_queries_lex::{Kind, Token};
use bson::Document;
use curlybrace_core::store::Query;

/// The text and its tokens, with small lookups that are `None` past the end.
pub(super) struct Cursor<'a> {
    pub text: &'a str,
    pub tokens: &'a [Token],
}

impl<'a> Cursor<'a> {
    pub(super) fn ident(&self, i: usize) -> Option<&'a str> {
        let token = self.tokens.get(i).filter(|t| t.kind == Kind::Ident)?;
        self.text.get(token.start..token.end)
    }

    pub(super) fn is_punct(&self, i: usize, b: u8) -> bool {
        self.tokens.get(i).is_some_and(|t| t.kind == Kind::Punct(b))
    }

    /// The name in `.name` starting at token `i`.
    fn dot_ident(&self, i: usize) -> Option<&'a str> {
        self.is_punct(i, b'.').then(|| self.ident(i + 1)).flatten()
    }

    /// `.name(` at token `i`: the name, and the closing `)` if it has one.
    fn dot_call(&self, i: usize) -> Option<(&'a str, Option<usize>)> {
        let name = self.dot_ident(i)?;
        let open = self
            .tokens
            .get(i + 2)
            .filter(|t| t.kind == Kind::Punct(b'('))?;
        Some((name, open.partner))
    }

    /// The text strictly between the tokens `open` and `close`.
    fn inner(&self, open: usize, close: usize) -> Option<&'a str> {
        let start = self.tokens.get(open)?.end;
        let end = self.tokens.get(close)?.start;
        self.text.get(start..end)
    }

    /// The argument texts between the brackets `open` and `close`, split at their own commas.
    fn args(&self, open: usize, close: usize) -> Option<Vec<&'a str>> {
        let mut spans = Vec::new();
        let mut start = self.tokens.get(open)?.end;
        let mut k = open + 1;
        while k < close {
            let token = self.tokens.get(k)?;
            match (token.kind, token.partner) {
                (Kind::Punct(b','), _) => {
                    spans.push(self.text.get(start..token.start)?);
                    start = token.end;
                    k += 1;
                }
                (Kind::Punct(b'(' | b'[' | b'{'), Some(partner)) if partner > k => k = partner + 1,
                _ => k += 1,
            }
        }
        spans.push(self.text.get(start..self.tokens.get(close)?.start)?);
        Some(spans)
    }
}

/// What a `db` token starts.
pub(super) enum Outcome {
    /// Not one of the collection calls read.
    Nothing,
    /// One of them, but not readable as literals.
    Unchecked,
    Call(Box<ShellCall>),
}

pub(super) struct Found {
    /// The database later calls use, when this expression names one (`None` inside: not a
    /// literal name).
    pub switch: Option<Option<String>>,
    pub outcome: Outcome,
}

/// The call that starts at the `db` token `t`, read against the database `current`
/// (`None`: not known).
pub(super) fn read_db(c: &Cursor<'_>, t: usize, current: &Option<String>) -> Found {
    let (db, after, switch) = match c.dot_call(t + 1) {
        Some(("getSiblingDB", Some(close))) => {
            let name = c.inner(t + 3, close).and_then(parse_string);
            (name.clone(), close + 1, Some(name))
        }
        Some(("getSiblingDB", None)) => {
            return Found {
                switch: Some(None),
                outcome: Outcome::Nothing,
            };
        }
        _ => (current.clone(), t + 1, None),
    };
    let outcome = match receiver(c, after) {
        None => Outcome::Nothing,
        Some(receiver) => call(c, t, db, receiver),
    };
    Found { switch, outcome }
}

/// A collection and method, with the method's `(` token and its partner.
struct Receiver {
    /// `None` when `getCollection` was given something other than a string literal.
    collection: Option<String>,
    method: &'static str,
    open: usize,
    close: Option<usize>,
}

fn method_named(name: &str) -> Option<&'static str> {
    METHODS.iter().find(|&&m| m == name).copied()
}

/// The receiver and method from token `p` (just after `db`, or after `getSiblingDB(...)`).
fn receiver(c: &Cursor<'_>, p: usize) -> Option<Receiver> {
    if let Some(("getCollection", close)) = c.dot_call(p) {
        let close = close?;
        let collection = c.inner(p + 2, close).and_then(parse_string);
        let (name, method_close) = c.dot_call(close + 1)?;
        return Some(Receiver {
            collection,
            method: method_named(name)?,
            open: close + 3,
            close: method_close,
        });
    }
    let mut names: Vec<&str> = Vec::new();
    let mut q = p;
    loop {
        if let Some((name, close)) = c.dot_call(q) {
            if names.is_empty() {
                return None;
            }
            return Some(Receiver {
                collection: Some(names.join(".")),
                method: method_named(name)?,
                open: q + 2,
                close,
            });
        }
        names.push(c.dot_ident(q)?);
        q += 2;
    }
}

/// The call read as literals, or `Unchecked` when any part that shapes the query is not one.
fn call(c: &Cursor<'_>, t: usize, db: Option<String>, receiver: Receiver) -> Outcome {
    let read = || -> Option<ShellCall> {
        let close = receiver.close?;
        let args = c.args(receiver.open, close)?;
        let filter = optional_literal(args.first().copied())?.unwrap_or_default();
        let finds = matches!(receiver.method, "find" | "findOne");
        let projection = if finds {
            optional_literal(args.get(1).copied())?
        } else {
            None
        };
        let chained = chain(c, close, finds)?;
        Some(ShellCall {
            db: db.clone()?,
            collection: receiver.collection.clone()?,
            method: receiver.method,
            query: Query {
                filter,
                sort: chained.sort,
                projection,
                skip: chained.skip,
                limit: chained.limit,
            },
            range: c.tokens.get(t)?.start..chained.end,
        })
    };
    read().map_or(Outcome::Unchecked, |call| Outcome::Call(Box::new(call)))
}

/// An absent or blank argument is `Some(None)`, a literal `Some(Some(doc))`, else `None`.
fn optional_literal(arg: Option<&str>) -> Option<Option<Document>> {
    match arg {
        Some(text) if !is_blank(text) => parse_literal(text).map(Some),
        _ => Some(None),
    }
}

struct Chained {
    sort: Option<Document>,
    skip: u64,
    limit: u64,
    /// Byte end of the last call in the chain.
    end: usize,
}

/// The `.name(...)` calls chained after the call closing at token `close`; `None` when one is
/// left open or, after `find`/`findOne`, its `sort` is not a literal.
fn chain(c: &Cursor<'_>, close: usize, finds: bool) -> Option<Chained> {
    let mut chained = Chained {
        sort: None,
        skip: 0,
        limit: 0,
        end: c.tokens.get(close)?.end,
    };
    let mut p = close + 1;
    while let Some((name, closing)) = c.dot_call(p) {
        let closing = closing?;
        let inner = c.inner(p + 2, closing)?;
        let count = || inner.trim().parse::<u64>().ok();
        chained = match name {
            "sort" if finds => Chained {
                sort: Some(parse_literal(inner)?),
                ..chained
            },
            "skip" if finds => Chained {
                skip: count().unwrap_or(chained.skip),
                ..chained
            },
            "limit" if finds => Chained {
                limit: count().unwrap_or(chained.limit),
                ..chained
            },
            _ => chained,
        };
        chained = Chained {
            end: c.tokens.get(closing)?.end,
            ..chained
        };
        p = closing + 1;
    }
    Some(chained)
}
