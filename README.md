# hackernews-mcp

A small, read-only [Model Context Protocol](https://modelcontextprotocol.io) server that gives
Claude Desktop and Claude Code first-class access to Hacker News. It speaks MCP over **stdio** and
talks to two public, no-auth APIs:

- **Official HN (Firebase)** — `https://hacker-news.firebaseio.com/v0/` — feeds, items, and users.
- **Algolia HN Search** — `https://hn.algolia.com/api/v1/` — full-text search.

No API keys, no accounts, nothing is mutated. Every tool is marked read-only.

## Tools

| Tool | When to reach for it |
| --- | --- |
| **`get_stories`** | Browse a ranked feed. Returns a compact list (id, title, author, score, comment count, url, `hn_url`) with **no** comment trees — ideal for scanning the front page or a section. Params: `category` (`top`/`new`/`best`/`ask`/`show`/`job`, default `top`), `limit` (default 30, max 100). |
| **`get_item`** | Go deep on **one** thread. Returns the item plus a nested comment tree walked breadth-first. The tree is hard-capped by `max_comments` (default 50) and `max_depth` (default 3) so it never blows up your context; a `truncation_note` tells you when replies were cut. Dead/deleted comments are skipped. Params: `id` (required), `include_comments` (default true), `max_depth` (default 3), `max_comments` (default 50). |
| **`get_user`** | Look up a profile by username (case-sensitive). Returns karma, the about text, account creation date, and submitted-item count. Param: `id` (required). |
| **`search`** | Full-text search across all of HN via Algolia. Params: `query` (required), `sort` (`relevance` default, or `date`), `tags` (e.g. `story`, `comment`, `ask_hn`, `show_hn`, `front_page`), `min_points` (int), `limit` (default 20, max 100). Returns hits with title, author, points, comment count, date, story URL, and `hn_url`. |

Two conventions across all tools: every Unix timestamp is returned both raw (`time`) and as an ISO 8601
UTC string (`time_iso`), and every item carries a derived `hn_url`
(`https://news.ycombinator.com/item?id=<id>`).

Quick rule of thumb: **`get_stories`/`search` to find things, `get_item` to read one thing in depth.**

## Build

Requires a recent stable Rust toolchain (edition 2021).

```sh
git clone https://github.com/devrelopers/hackernews-mcp
cd hackernews-mcp
cargo build --release
```

The binary lands at `target/release/hackernews-mcp`.

## Install in Claude Desktop

Add an entry to your `claude_desktop_config.json`:

- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "hackernews": {
      "command": "/absolute/path/to/hackernews-mcp/target/release/hackernews-mcp"
    }
  }
}
```

Use the absolute path to the binary you just built (e.g.
`/Users/you/hackernews-mcp/target/release/hackernews-mcp`). Restart Claude Desktop; the four tools
will appear under the 🔌 tools menu.

## Install in Claude Code

```sh
claude mcp add hackernews -- /absolute/path/to/hackernews-mcp/target/release/hackernews-mcp
```

## Notes

- Item fetches in `get_stories` and `get_item` run concurrently with a politeness cap of ~10
  in-flight requests.
- Logs are written to **stderr** so they never corrupt the JSON-RPC stream on stdout. Set
  `RUST_LOG=debug` for verbose tracing.
- Missing items/users return an actionable error
  (`Item <id> not found — it may be deleted or the ID may be invalid.`) rather than a panic.

## Layout

```
src/
  main.rs     # tokio entrypoint + stdio transport wiring
  client.rs   # shared reqwest client over the Firebase + Algolia APIs
  tools.rs    # the four MCP tools + breadth-first comment-tree walker
  types.rs    # data models and compact output shapes
```

## License

MIT — see [LICENSE](LICENSE).
