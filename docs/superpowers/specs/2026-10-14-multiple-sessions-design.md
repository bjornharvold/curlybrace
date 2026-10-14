# Several simultaneous connections: Design

Date: 2026-10-14
Status: Scope, runtime shape, session identity, main-pane behaviour and manager access approved by the
maintainer on 2026-10-14
Issue: #46 (slice 2 of #45). Related: #38 (parity map), #8 (Data Compare), #47, #48.
Builds on: `2026-10-14-connection-manager-design.md` (slice 1).

## Context and intent

Slice 1 delivered the saved-connection store and manager, but the app can still browse only one server at a
time: `Shared` holds one store, one tunnel and one epoch, and `AppState` holds one set of browsing fields.
This slice lets several saved connections be open at once, each with its own tree root, status and view, so
that slice 3 (copy data between connections) has two live sessions to work with.

Public behaviour only is referenced (ground rules in #38); layout and wording are our own.

## Decisions

1. **One runtime, a session map.** The store, tunnel, generation and `epoch_lock` of `Shared` move into a
   `Session`; `Shared` keeps the connector, secrets, config and the Compare registry, plus
   `sessions: RwLock<HashMap<SessionId, Arc<Session>>>`. Compare and Test Connection stay independent of it.
2. **One session per saved connection.** `SessionId` is the connection's `Uuid`. Connect on an open
   connection focuses it; it never opens a second session, tunnel or host-key prompt.
3. **A view per connection.** Each session keeps its own selected collection, query, page and editor. A
   header always names the connection.
4. **The manager is a workspace.** A Connections button above the tree switches the main area to the
   existing manager screen. Connect opens (or focuses) the session and returns to Browse.
5. **Scope:** sessions, tree roots and per-session status and views. Not in this slice: Compare using open
   sessions (slice 3), search across connections (follow-up), read-only and colour tags (#47), several
   sessions per connection, a shell tab.

## Runtime (core)

```
Shared { sessions, next_epoch: AtomicU64, secrets, connector, config, compare }
Session { store, tunnel, generation: AtomicU64, epoch_lock }
```

- **Epochs are allocated from one global counter** (`next_epoch`) and stored per session as that session's
  current generation. Every `Connect` and `Disconnect` command takes the next number, whichever session it
  names, so numbers never repeat and a late event of a closed session can never look current to a new one.
  The UI mirrors the counter exactly as it mirrors it today (it is the only sender).
- `Connect(cfg)`: the session id is `cfg.id`. The dispatcher synchronously gets or creates the `Session`,
  takes the next epoch and drops that session's store and tunnel (`begin_epoch`), so request order decides
  within one session. Other sessions are untouched.
- `Disconnect { session }`: bumps that session's epoch, drops its store and tunnel, then removes it from the
  map. Tasks still running hold their own `Arc<Session>` and see a stale generation, so their answers become
  `Superseded`.
- Data commands (`ListDatabases`, `ListCollections`, `Find`, `Insert`, `Replace`, `Delete`, `Clone`) carry
  `session`. A command for an unknown or closed session answers `Failed { error: Unsupported("not
  connected") }` stamped with that session. It is never routed anywhere else.
- Every session event carries `session`: `Connected`, `Disconnected`, `ConnectFailed`, `Databases`,
  `Collections`, `Results`, `Mutated`, `Failed`. `Event::Compare` and `Event::TestConnection` are unchanged.
- `Connected` also carries `topology: Option<Topology>` (`Direct`, `ReplicaSet`, `Sharded`), read once from
  the server's `hello` reply by `DocumentStore::topology()` (default `Ok(None)`; a failure is `None`).
- The final check-and-send of a data answer (`send_answer`) uses the answering session's `epoch_lock`.

## UI state

Existing code works on the browsing fields of `AppState` (about 300 references). They stay as the **active
session's view** so every existing reducer path and test keeps working. New on top:

- `open: Vec<Uuid>`: open sessions in opening order (the tree order); `active: Option<Uuid>` (replaces
  `connected`).
- `parked: BTreeMap<Uuid, SessionView>`: the saved view of every open session that is not active.
  `SessionView` holds `status`, `latest_requested_epoch`, `topology`, `failure`, `sidebar`, `loading`, `find`,
  `issued_query`, `page_query`, `loading_page`, `page`, `page_key`, `tree`, `view_mode`, `editor`, `pending`
  and `ssh_endpoint`, i.e. exactly the per-session fields.
- `epoch_counter: u64` mirrors the runtime's global counter.
- Pure, immutable operations: `activated(id)` parks the current view and restores the target's;
  `with_session(id, f)` applies `f` to a session's view (swapping it in and back, so no logic is duplicated);
  `session_dropped(id)`; `roots()` yields `(id, name, status, topology)` for the tree.
- `SessionStatus` gains `Failed`: the root stays in the tree with its error and a Reconnect action.
  `pending` is part of the view, so a slow write on one session never blocks another.

### Event routing

Each session event goes to the active view if its session is active, to `with_session` if the session is
open but parked, and is dropped otherwise (closed or unknown). `Results` is accepted only when the session
matches AND the namespace and query match, so two connections that both have `app.users` can never overwrite
each other's page. Commands built inside a reducer path carry the session that path is working on.

### Flows

- **Connect(id)** on a closed or failed connection: park the active view, create a fresh view for `id`
  (status `Connecting`, epoch from the counter), make it active, send `Connect`. On an open connection:
  `activated(id)` only.
- **Disconnect(id)**: send `Disconnect { session }`, drop the view. If it was active, activate the
  neighbour (the next open root, else the previous); with none left, show the manager.
- **Failure** of a connect or a lost session marks only that root `Failed`; nothing else changes.
- **Reconnect** is an explicit action on a failed root (same as Connect). There is no automatic reconnect
  beyond the driver's own.
- `last_connected` is stamped per successful connect, for that connection.

## UI

- Sidebar: one root per open session (name, topology badge, status). Roots expand, collapse and select
  independently; each has Disconnect, and Reconnect when failed. Opening a collection under a parked root
  activates it; expanding or collapsing a database does not.
- Header of the documents pane names the active connection. Delete and replace confirmations name it too.
- Connections button above the tree switches `Workspace` to `Connections` (the manager screen).
- Host-key prompts keep the existing queue (one visible, the rest behind it). The dialog names the host and,
  when exactly one connecting session uses that endpoint, its connection.
- Compare's source default is the active session. Compare still opens its own connections.
- Every string is an i18n key, with keyboard access and accessibility tests.

## Security

No URI, password or passphrase reaches a log, event, banner or `Debug` output (slice 1 rules). A session's
tunnel is opened and closed with its session, host-key verification still fails closed, and a prompt answers
only for the connection that asked. Writes carry the session id; a write for a session that is not open
fails closed instead of being applied elsewhere.

## Testing

Core (fake connector): two sessions connected at once; connect, disconnect or failure of one leaves the
other's store, tunnel and in-flight answers alone; stale answers are dropped per session; an unknown session
fails closed; epochs never repeat across sessions. Reducer: connect, focus, disconnect with neighbour
selection, failed root, per-session view restore, same namespace on two sessions, late events of a closed
session. Snapshots of the tree with several roots in different states; accessibility tests. Integration
(tier 2): two real MongoDB containers open at once, one stopped, the other keeps answering.

## Out of scope

See decision 5. Also: persistent "reopen last connections on start" (start screen, #82).
