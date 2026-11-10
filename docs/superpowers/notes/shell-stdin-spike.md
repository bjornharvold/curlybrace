# Shell tab: mongosh secret delivery spike

Date: 2026-11-10. Tool: `mongosh` 2.14.1 (macOS, Homebrew). Server: a throwaway local `mongod`
with `--auth` on 127.0.0.1:27999, user `spike` on `admin` with password `p@ss:w/rd%#?`
(percent-encoded in the URI as `p%40ss%3Aw%2Frd%25%23%3F`). The instance and its dbpath were
deleted afterwards. No other MongoDB was touched.

## Decision

```
Delivery::Stdin
```

The connect line is written to the stdin of a long-lived `mongosh --nodb --quiet` process. The
password lives only in that one stdin line. It is not in argv, not in the child's environment,
not in mongosh's log and not in mongosh's REPL history. Methods 2 (`/dev/fd/N`) and 3 (private
0600 file) also work and keep the password out of argv, but they were not needed. They are
recorded below as fallbacks, with their different behaviour.

## Command line

```
mongosh --nodb --quiet
```

Then the first stdin line is:

```
db = connect("mongodb://USER:ENCODED_PW@HOST:PORT/AUTHDB")
```

`connect()` works and returns a `Db`; `db` is then the connected database (the one named in the
URI path, here `admin`). `Mongo("<uri>")` also works (`const m = Mongo(uri); m.getDB("admin")`),
and returned `authenticatedUsers: [{user:'spike', db:'admin'}]`. Prefer `db = connect(...)`: the
shell then behaves like a normal `mongosh <uri>` session (`db`, `show` helpers, prompt shows the db
name). The password must be percent-encoded in the URI (user and password both).

## Findings

1. Stdin is evaluated as a REPL even when stdin is not a TTY, and the result of each bare
   expression is printed (exactly like an interactive session).
2. Long-lived streaming works. Writing one line at a time to a pipe and reading the output as it
   arrives works. Each statement's output is followed by the prompt, which is a usable
   end-of-output marker (see "Output format").
3. All output, including errors, goes to stdout. stderr stays empty. The exit status is 0 even
   after an authentication failure, a syntax error or a thrown error. With stdin the shell never
   exits on error; it prints the error and waits for the next line. The process exits 0 on EOF.
   So callers cannot rely on exit status or stderr in this mode.
4. `--json` cannot be combined with stdin:
   `MongoshRuntimeError: Cannot use --json without --eval or with --shell or with extra files`
   (exit 1). The flag exists (`--json[=canonical|relaxed]`) but only for `--eval`, which would put
   the script in argv. It is not usable for the shell tab. Output must be parsed as the
   human-formatted shell output.
5. Nothing sensitive is persisted by mongosh. See "Leak checks".

## Output format (raw bytes, `--nodb --quiet`, stdin not a TTY)

The shell prints a prompt before reading each line. Before connecting the prompt is `> `. After
`db = connect(".../admin")` it is `admin> ` (database name, so it changes after `use x`).

| Input | Bytes on stdout (prompt shown as `P>`) |
| --- | --- |
| `db = connect(uri)` | `> ` then `admin\nadmin> ` (echoes the db name, then next prompt) |
| `1+1` | `2\nP> ` |
| `"e"` | `e\nP> ` (no quotes) |
| `({a:1,b:[1,2]})` | `{ a: 1, b: [ 1, 2 ] }\nP> ` (pretty printed, multi-line when long) |
| `db.runCommand({ping:1})` | `{ ok: 1 }\nP> ` |
| `print("x")` | `x\n\nP> ` (the text, then an extra blank line because `print` returns undefined and the shell prints an empty result line) |
| `sleep(1500); "slept"` | `slept\nP> ` after the delay |

So a bare expression gives `value\n` and `print()` gives `text\n\n`.

Error lines are on stdout, prefixed `Uncaught ` followed by a carriage return before the error
name (the shell redraws the line), for example:

```
Uncaught \rMongoServerError[AuthenticationFailed]: Authentication failed.\n> 
```

- Authentication failure (wrong password), first line: `> Uncaught \rMongoServerError[AuthenticationFailed]: Authentication failed.\n> `.
  The prompt stays `> ` (no database name) because no connection exists. Later commands that use
  `db` print `Uncaught \rMongoshInvalidInputError: [SHAPI-10004] No connected database`. Pure
  expressions (`1+1`, `print`) still work. Exit status 0, stderr empty.
- Syntax error (`var = = 3`): stdout gets `Uncaught:\n\rSyntaxError: Unexpected token (1:4)\n\n> 1 | var = = 3\n    |     ^\n  2 |\n\n` then the prompt. Exit 0, stderr empty. The shell keeps going; the next line is evaluated.
  Note that `1+` is not a syntax error in the REPL: it waits for a continuation line, so the next
  statement is consumed into the expression (`1+print("after")` gave `after` then `NaN`). The UI
  must be ready for continuation (multi-line) input; the prompt does not change visibly in
  `--quiet` non-TTY mode, so treat each submitted block as one unit.
- Runtime error (`throw new Error("boom")`): `Uncaught \rError: boom\n` then the prompt. Exit 0.

Detecting connect success without a prompt parse: the prompt after the connect line is
`<db>> ` on success and `> ` on failure; or check the first output for `Uncaught`. A more robust
variant for Task 3 is to send the connect line wrapped so that it prints a sentinel
(for example `try { db = connect(uri); print("__OK__") } catch (e) { print("__ERR__ " + e.message) }`),
but this was not exercised in the spike.

## Leak checks (password `p@ss:w/rd%#?`)

- argv (`ps -ww -o pid,command -p <pid>`) during `sleep(6000)` for all three methods:
  `mongosh` followed by spaces only. The URI is never in argv for stdin, fd or file mode.
  Caveat: mongosh rewrites its own process title, so this check is weak by itself. A control run
  that passes the URI in argv showed `mongosh mongodb://<credentials>@127.0.0.1:...` (mongosh
  scrubs the title, but the real argv would still be visible to a very early `ps` or the kernel
  and in the parent's own spawn logs). The guarantee is by construction: the connect line is never
  placed in argv.
- environment (`ps eww -p <pid>`): output was just the 74-byte title with no environment, and a
  control run with `MONGOSH_PW='p@ss:w/rd%#?'` set also showed nothing (count 0). So on macOS
  with mongosh's rewritten title, `ps eww` cannot show the environment at all and this check
  proves nothing by itself. `/proc/<pid>/environ` does not exist on macOS. By construction the
  spawner sets no secret in the environment; the password appears in no env var. Task 3 must
  assert this in a unit test on the `Command` it builds (no `MONGOSH*`/password env, argv equals
  `["--nodb", "--quiet"]`).
- mongosh log (`~/.mongodb/mongosh/<id>_log`): `grep` for the encoded and plain password in
  all of `~/.mongodb` found nothing. The log records `uri: "mongodb://<credentials>@127.0.0.1:27999/admin?directConnection=true..."`
  (credentials redacted). The connect line itself is not logged as "Evaluating input". Subsequent
  stdin lines ARE logged as `Evaluating input` (for example the `"SPIKEMARK"` line was found
  in the log), so user-typed commands containing secrets (for example `db.createUser({pwd:...})`)
  end up in that log. This is the user's own mongosh behaviour; consider `disableLogging` via
  `config.set` or document it.
- REPL history (`~/.mongodb/mongosh/mongosh_repl_history`): the file is opened in stdin mode, but
  none of the marker strings, `connect(`, the password or `sleep(6000)` were written to it. Stdin
  lines are not added to history when stdin is not a TTY.
- mongod server log: no password (0 matches).
- Open file descriptors of the running process (`lsof`): stdin is a `PIPE`; no secret file is open.

## Fallbacks (not selected)

Method 2, script as `/dev/fd/N` (bash/zsh `<(...)`: `mongosh --nodb --quiet /dev/fd/N`): works,
argv shows only the fd path. This is file/script mode, not a REPL, so it behaves differently:
- no prompt, and bare expressions are NOT echoed (only `print`/`printjson` output appears:
  `p`, `{\n  a: 1\n}`);
- errors go to stderr and the exit status is 1: auth failure `MongoServerError: Authentication failed.`,
  syntax error `SyntaxError: Unexpected token (1:4) ...`, runtime error `Error: boom`;
- the process runs the script and exits, so it cannot take interactive input afterwards.
It is suitable for one-shot scripts, not for an interactive shell.

Method 3, 0600 file in a 0700 directory (`umask 077`): `mongosh --nodb --quiet <dir>/s.js`
behaves the same as method 2 (stderr + exit 1 on errors, no echo), and the password sits on disk
until deleted. Not selected.

## Implications for Task 3

- Spawn `mongosh --nodb --quiet` with piped stdin/stdout/stderr, a minimal environment (do not
  forward secrets) and `kill_on_drop`.
- First write the connect line (URL-encode user and password), then stream the user's input.
  Overwrite or drop the in-memory line afterwards.
- Parse stdout using the prompt (`^[A-Za-z0-9_.-]*> $` at the end of a read) as the
  "command finished" marker; expect `Uncaught \r` markers for errors and treat the exit status as
  unreliable (only meaningful when the process dies).
- Strip the `\r` that follows `Uncaught `.
- No `--json`. Output is human-formatted text.

## Final method

What shipped (Tasks 3 and 4), after the spike above:

- **Stdin, REPL.** `mongosh --nodb --quiet` with piped stdin; the secret is never in argv or the
  environment. `--json` cannot be used with stdin (finding 4), so output is the human-formatted text
  and slice D (table/tree results) needs another mechanism.
- **Fixed-prompt preamble.** The first stdin line sets `prompt = () => "\u0001CBP> "` (the prompt
  is then the same whatever the database is, and ends every command), then runs `try { db =
  connect(...) ... }` with sentinels (`__CB_READY__`, or `__CB_FAILED__ <message>`) so that success and
  failure of the connect are told apart without parsing a prompt. The prompt marker and the
  sentinels are removed from the streamed output.
- **Handshake.** The script is sent only after the ready sentinel arrived. A failed connect, an
  early exit or no answer within the handshake timeout ends the run as `ConnectFailed`, and the
  script is never sent.
- **Errors come on stdout with exit 0** (finding 3). The runner classifies them by the `Uncaught`
  marker: such a run ends `ScriptErrors` and the error text is shown on the stderr stream. A run
  without them ends `Exited(0)`. The exit status alone says nothing in this mode.
- Checked against a real `mongosh` by the gated test `crates/core/tests/local_shell.rs`.
