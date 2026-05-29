//! MCP tool definitions. Each tool is read-only and talks to the public HN
//! APIs through [`HnClient`]. Output is compact JSON tuned for an LLM context
//! window — most importantly, `get_item` hard-caps the comment tree it returns.

use std::collections::HashMap;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;

use crate::client::{Category, HnClient, SearchSort};
use crate::types::{
    unix_to_iso, CommentNode, ItemDetail, SearchHit, StorySummary, UserProfile,
};

const DEFAULT_STORY_LIMIT: u32 = 30;
const MAX_STORY_LIMIT: u32 = 100;
const DEFAULT_SEARCH_LIMIT: u32 = 20;
const MAX_SEARCH_LIMIT: u32 = 100;
const DEFAULT_MAX_DEPTH: u32 = 3;
const DEFAULT_MAX_COMMENTS: usize = 50;

// ---------------------------------------------------------------------------
// Tool argument schemas
// ---------------------------------------------------------------------------

/// Which ranked story feed to pull.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CategoryArg {
    Top,
    New,
    Best,
    Ask,
    Show,
    Job,
}

impl From<CategoryArg> for Category {
    fn from(c: CategoryArg) -> Self {
        match c {
            CategoryArg::Top => Category::Top,
            CategoryArg::New => Category::New,
            CategoryArg::Best => Category::Best,
            CategoryArg::Ask => Category::Ask,
            CategoryArg::Show => Category::Show,
            CategoryArg::Job => Category::Job,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetStoriesArgs {
    /// Story feed to read. One of: top, new, best, ask, show, job. Default: top.
    pub category: Option<CategoryArg>,
    /// How many stories to return. Default 30, max 100.
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetItemArgs {
    /// The numeric HN item ID.
    pub id: u64,
    /// Include the comment tree. Default: true.
    pub include_comments: Option<bool>,
    /// How many levels of replies to descend. Default: 3.
    pub max_depth: Option<u32>,
    /// Hard cap on total comments fetched, to protect context. Default: 50.
    pub max_comments: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetUserArgs {
    /// The HN username (case-sensitive).
    pub id: String,
}

/// How Algolia should rank search results.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortArg {
    Relevance,
    Date,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// Free-text query.
    pub query: String,
    /// Ranking: "relevance" (default) or "date" (newest first).
    pub sort: Option<SortArg>,
    /// Algolia tag filter, e.g. "story", "comment", "ask_hn", "show_hn",
    /// "front_page", or "author_pg". Combine with commas for AND.
    pub tags: Option<String>,
    /// Only return hits with at least this many points.
    pub min_points: Option<i64>,
    /// Max hits to return. Default 20, max 100.
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct HackerNews {
    client: HnClient,
    tool_router: ToolRouter<HackerNews>,
}

#[tool_router]
impl HackerNews {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: HnClient::new()?,
            tool_router: Self::tool_router(),
        })
    }

    #[tool(
        description = "Browse a ranked list of Hacker News stories (top/new/best/ask/show/job). \
            Returns compact rows (id, title, author, score, comment count, url, hn_url) with NO \
            comment trees. Use this to scan the front page or a feed; then call get_item with an id \
            to read one thread in depth.",
        annotations(
            title = "Get HN stories",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_stories(
        &self,
        Parameters(args): Parameters<GetStoriesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let category: Category = args.category.unwrap_or(CategoryArg::Top).into();
        let limit = args.limit.unwrap_or(DEFAULT_STORY_LIMIT).clamp(1, MAX_STORY_LIMIT) as usize;

        let mut ids = self.client.story_ids(category).await.map_err(internal)?;
        ids.truncate(limit);

        let items = self.client.items_in_order(&ids).await;
        let stories: Vec<StorySummary> = items.into_iter().map(StorySummary::from_item).collect();
        ok_json(&stories)
    }

    #[tool(
        description = "Go deep on a single Hacker News item by id (story, comment, job, or poll). \
            Returns the item plus a nested comment tree, breadth-first. The tree is hard-capped by \
            max_comments (default 50) and max_depth (default 3) to protect context — a \
            truncation_note is included when replies were cut. Set include_comments=false for just \
            the item metadata. Use get_stories or search first to find an id.",
        annotations(
            title = "Get HN item with comments",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_item(
        &self,
        Parameters(args): Parameters<GetItemArgs>,
    ) -> Result<CallToolResult, McpError> {
        let item = self
            .client
            .item(args.id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                McpError::resource_not_found(
                    format!(
                        "Item {} not found — it may be deleted or the ID may be invalid.",
                        args.id
                    ),
                    None,
                )
            })?;

        let root_kids = item.kids.clone().unwrap_or_default();
        let mut detail = ItemDetail::from_item(item);

        let include_comments = args.include_comments.unwrap_or(true);
        if include_comments && !root_kids.is_empty() {
            let max_depth = args.max_depth.unwrap_or(DEFAULT_MAX_DEPTH).max(1);
            let max_comments = args.max_comments.unwrap_or(DEFAULT_MAX_COMMENTS);
            let (comments, note) = self
                .build_comment_tree(root_kids, max_depth, max_comments)
                .await;
            detail.comments = Some(comments);
            detail.truncation_note = note;
        }

        ok_json(&detail)
    }

    #[tool(
        description = "Look up a Hacker News user profile by username (case-sensitive). Returns \
            karma, the about/bio text, account creation date, and how many items they've submitted.",
        annotations(
            title = "Get HN user",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn get_user(
        &self,
        Parameters(args): Parameters<GetUserArgs>,
    ) -> Result<CallToolResult, McpError> {
        let user = self
            .client
            .user(&args.id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                McpError::resource_not_found(
                    format!(
                        "User '{}' not found — usernames are case-sensitive.",
                        args.id
                    ),
                    None,
                )
            })?;
        ok_json(&UserProfile::from_user(user))
    }

    #[tool(
        description = "Full-text search across all of Hacker News via Algolia. Use this to find \
            stories or comments by keyword, author, or popularity — sort by 'relevance' (default) \
            or 'date'. Filter with tags (story, comment, ask_hn, show_hn, front_page) and min_points. \
            Returns hits with title, author, points, comment count, date, and URLs.",
        annotations(
            title = "Search HN",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let sort = match args.sort.unwrap_or(SortArg::Relevance) {
            SortArg::Relevance => SearchSort::Relevance,
            SortArg::Date => SearchSort::Date,
        };
        let limit = args
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);

        let numeric_filters = args.min_points.map(|p| format!("points>={p}"));

        let resp = self
            .client
            .search(
                &args.query,
                sort,
                args.tags.as_deref(),
                numeric_filters.as_deref(),
                limit,
            )
            .await
            .map_err(internal)?;

        let hits: Vec<SearchHit> = resp.hits.into_iter().map(SearchHit::from_hit).collect();
        ok_json(&hits)
    }

    /// Breadth-first walk of a comment tree, fetching each level concurrently
    /// and stopping at `max_depth` or once `max_comments` are collected.
    /// Returns the assembled (nested) tree and an optional truncation note.
    async fn build_comment_tree(
        &self,
        root_kids: Vec<u64>,
        max_depth: u32,
        max_comments: usize,
    ) -> (Vec<CommentNode>, Option<String>) {
        struct Raw {
            by: Option<String>,
            text: Option<String>,
            time: Option<i64>,
            depth: u32,
            kids: Vec<u64>,
        }

        let mut collected: HashMap<u64, Raw> = HashMap::new();
        let mut frontier = root_kids.clone();
        let mut depth = 1u32;
        let mut overflow = 0usize;
        let mut truncated = false;

        while !frontier.is_empty() && depth <= max_depth {
            let remaining = max_comments.saturating_sub(collected.len());
            if remaining == 0 {
                overflow += frontier.len();
                truncated = true;
                break;
            }
            let take = frontier.len().min(remaining);
            if take < frontier.len() {
                overflow += frontier.len() - take;
                truncated = true;
            }
            let to_fetch: Vec<u64> = frontier[..take].to_vec();
            let items = self.client.items_in_order(&to_fetch).await;

            let mut next = Vec::new();
            for item in items {
                if item.deleted || item.dead {
                    continue;
                }
                let kids = item.kids.clone().unwrap_or_default();
                if depth < max_depth {
                    next.extend(kids.iter().copied());
                } else if !kids.is_empty() {
                    truncated = true;
                }
                collected.insert(
                    item.id,
                    Raw {
                        by: item.by,
                        text: item.text,
                        time: item.time,
                        depth,
                        kids,
                    },
                );
            }
            frontier = next;
            depth += 1;
        }

        fn assemble(id: u64, map: &HashMap<u64, Raw>) -> Option<CommentNode> {
            let raw = map.get(&id)?;
            let replies = raw
                .kids
                .iter()
                .filter_map(|k| assemble(*k, map))
                .collect();
            Some(CommentNode {
                id,
                by: raw.by.clone(),
                text: raw.text.clone(),
                time: raw.time,
                time_iso: raw.time.and_then(unix_to_iso),
                depth: raw.depth,
                replies,
            })
        }

        let tree: Vec<CommentNode> = root_kids
            .iter()
            .filter_map(|id| assemble(*id, &collected))
            .collect();

        let note = if truncated {
            if overflow > 0 {
                Some(format!(
                    "{overflow} more comments not shown (limited by max_comments={max_comments}, max_depth={max_depth})"
                ))
            } else {
                Some(format!(
                    "Some deeper replies not shown (max_depth={max_depth} reached)"
                ))
            }
        } else {
            None
        };

        (tree, note)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HackerNews {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Read-only Hacker News tools backed by the official Firebase API and Algolia \
                 search. Use get_stories to browse a feed, search to find items by keyword, \
                 get_item to read one thread (comment tree is capped to protect context), and \
                 get_user for a profile."
                    .to_string(),
            )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialize a value to pretty JSON and wrap it as tool text content.
fn ok_json<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("serializing response: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

/// Map an internal error to an MCP error with a full cause chain.
fn internal(e: anyhow::Error) -> McpError {
    McpError::internal_error(format!("{e:#}"), None)
}
