# Shell completion and syntax check

Issue [#50](https://github.com/bjornharvold/curlybrace/issues/50), slice E (the last one). Builds on
the Shell tab ([design](2026-11-10-shell-tab-design.md)). The design below is the author's call:
the user opted out of the question round ("Just start"), so every choice here is marked as a
default to be corrected on review.

## Why

The Shell tab is an editor, and an editor should help while you type. Two things the issue asks
for: suggestions for collection names and the usual operators, and a warning about a broken script
before `mongosh` is started and fails with a line of noise.

## What it does

**Completion.** While typing in the editor, a small list appears under the caret with names that
fit the word being typed. Up and Down move in the list, Enter or Tab inserts, Esc closes it.
`Ctrl+Space` opens it on demand. Completion is offered only in code, never inside a string,
template text, comment or regular expression. It is suggestions only: nothing is inserted until
the user picks one.

| Where the caret is | What is offered |
|---|---|
| a word at the start of a statement | `db`, `use`, `show`, `sh`, `rs`, `it`, `print`, `printjson`, `ObjectId`, `ISODate`, `NumberLong`, ... |
| after `show ` | `dbs`, `databases`, `collections`, `users`, `roles`, `profile`, `logs` |
| after `db.` | the collections of the loaded databases, then database methods (`getCollection`, `runCommand`, `getName`, `stats`, ...) |
| after `db.<name>.` | collection methods (`find`, `findOne`, `aggregate`, `insertOne`, `updateMany`, `countDocuments`, ...) |
| after `$` inside an object or array | query, update and aggregation operators (`$match`, `$group`, `$set`, `$in`, ...) |

Collection names come from what the sidebar already loaded for the active session (a database
whose collections were never loaded has none to offer; nothing is fetched just for completion).
They are the names of all loaded databases, since the script's current database (`use x`) is not
tracked: a limit, stated, not hidden. A name that is not a plain identifier is inserted as
`db.getCollection("name")`, replacing the `db.` the user typed.

**Syntax check.** Before a Run, the text that would run is checked locally, without starting
`mongosh`. It finds the problems that are certain from the shape of the text: a bracket that is
never closed, a closing bracket with nothing to close or of the wrong kind, a string, template
literal or block comment that never ends. The result names the problem and its line and column.
A problem blocks the Run and shows a notice with the message and two buttons: **Go to problem**
(moves the caret) and **Run anyway**. The check also marks the problem live: the editor status
line under the box shows the first problem while the user types (not blocking).

## Goals

- Pure, egui-free decision code (context at the caret, candidate list, syntax check) in the
  viewmodel, testable without a window.
- No new dependency: the existing lexer (`shell_highlight::tokens`) and the formatter's regex rule
  are reused, so highlight, format, completion and check agree on what is a string or a comment.
- Never lose or alter the user's text except by an explicit pick of a suggestion.
- Every user-visible string is an i18n key; completion list and notice pass the accessibility
  tests (the list is announced, the selected item is exposed).

## Non-goals

- A real JavaScript parser, type inference, completion of fields inside documents, completion
  from the server (`db.getCollectionNames()`), snippets, a debugger.
- Tracking `use <db>` to narrow collection names to one database.
- Catching errors that need a parser (a misspelled method, a missing comma between properties).
  A script the check passes can still fail in `mongosh`; the check says only what is certain.

## Anti-goals

- A check that blocks a valid script with no way out. Because the lexer treats a regex literal by
  heuristic, the block always offers **Run anyway**.
- Suggestions that appear in a way that steals Enter or Tab from a user who just wants a newline
  or an indent: the list takes those keys only while it is open and has a selected item, and it
  opens on typing a word character or `.`, never on whitespace.

## Decisions (defaults of the author)

- Completion is a popup list under the caret, not inline ghost text.
- Prefix match, case-insensitive, at most 12 items, collections first after `db.`.
- The check blocks with an escape hatch rather than only warning.
- The check runs on the text that would run (selection or all), not the whole editor, so a user
  can run a fine selection of a broken draft. The live status line checks the whole editor.
- Operator and method lists are fixed constants in the viewmodel (the mongosh API of the version
  floor the app requires); they are code names, not translated text.

## Left to the builder

Popup placement and styling within the existing theme; the exact message wording (as i18n keys
under `shell.complete.*` and `shell.check.*`); the constant lists' full contents beyond the
examples above.
