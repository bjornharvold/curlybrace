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

/// `.getSiblingDB(...)` at token `i`, closed: its name (`None`: not a string literal) and the
/// index of its closing `)`. `None` when there is no such closed call.
pub(super) fn sibling_db(c: &Cursor<'_>, i: usize) -> Option<(Option<String>, usize)> {
    match c.dot_call(i)? {
        ("getSiblingDB", Some(close)) => {
            Some((c.inner(i + 2, close).and_then(parse_string), close))
        }
        _ => None,
    }
}

/// The call that starts at the `db` token `t`, read against the database `current`
/// (`None`: not known). A `db.getSiblingDB(...)` receiver names the database of this call
/// only, as in mongosh; it never changes `current`, and while `current` is not known the
/// sibling is not either (`db` may be another server's). A listed method reached through any
/// other receiver shape is `Unchecked`, never dropped.
pub(super) fn read_db(c: &Cursor<'_>, t: usize, current: &Option<String>) -> Outcome {
    let start = match (c.dot_call(t + 1), sibling_db(c, t + 1)) {
        (_, Some((name, close))) => Some((current.as_ref().and(name), close + 1)),
        (Some(("getSiblingDB", None)), None) => None,
        _ => Some((current.clone(), t + 1)),
    };
    let read = start.and_then(|(db, after)| receiver(c, after).map(|r| call(c, t, db, r)));
    match read {
        Some(outcome) => outcome,
        None if reaches_method(c, t + 1) => Outcome::Unchecked,
        None => Outcome::Nothing,
    }
}

/// Whether the member chain from token `p` (`.name`, `.name(...)`, `[...]` segments) calls one
/// of [`METHODS`].
fn reaches_method(c: &Cursor<'_>, p: usize) -> bool {
    let mut q = p;
    loop {
        let next = match c.dot_call(q) {
            Some((name, _)) if method_named(name).is_some() => return true,
            Some((_, close)) => close.map(|close| close + 1),
            None if c.dot_ident(q).is_some() => Some(q + 2),
            None if c.is_punct(q, b'[') => c
                .tokens
                .get(q)
                .and_then(|t| t.partner)
                .map(|close| close + 1),
            None => None,
        };
        match next {
            Some(n) => q = n,
            None => return false,
        }
    }
}

/// `.getCollection(...)` or `[...]` at token `p`, closed: the collection (`None`: not a string
/// literal) and the index of the closing token.
fn collection_index(c: &Cursor<'_>, p: usize) -> Option<(Option<String>, usize)> {
    if let Some(("getCollection", close)) = c.dot_call(p) {
        let close = close?;
        return Some((c.inner(p + 2, close).and_then(parse_string), close));
    }
    if !c.is_punct(p, b'[') {
        return None;
    }
    let close = c.tokens.get(p)?.partner?;
    Some((c.inner(p, close).and_then(parse_string), close))
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
    if let Some((collection, close)) = collection_index(c, p) {
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
        // Options (hint, collation, ...) change the call's plan: not the query we explain.
        let options = args
            .iter()
            .skip(options_at(receiver.method))
            .any(|arg| !is_blank(arg));
        if options {
            return None;
        }
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

/// The position of `method`'s options argument: `find(filter, projection, options)`,
/// `updateX(filter, update, options)`, `countDocuments(filter, options)`,
/// `deleteX(filter, options)`.
fn options_at(method: &str) -> usize {
    match method {
        "find" | "findOne" | "updateOne" | "updateMany" => 2,
        _ => 1,
    }
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
/// left open, is a plan-changing `hint`/`collation`/`min`/`max`, or, after `find`/`findOne`,
/// is a `sort` that is not a literal.
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
            // These change the plan the server picks, so our explain would not match.
            "hint" | "collation" | "min" | "max" => return None,
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
