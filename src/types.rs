//! Data models for the official HN Firebase API and the Algolia search API,
//! plus the compact output shapes returned by each tool.
//!
//! Every upstream field is optional except `id`. Unknown fields are ignored
//! (serde drops them by default), so new HN fields never break deserialization.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// Convert a Unix timestamp (seconds) to an ISO 8601 UTC string with a `Z`
/// suffix, e.g. `2024-01-02T03:04:05Z`. Returns `None` for out-of-range values.
pub fn unix_to_iso(ts: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
}

/// Build the canonical Hacker News web URL for an item or user thread.
pub fn hn_item_url(id: u64) -> String {
    format!("https://news.ycombinator.com/item?id={id}")
}

// ---------------------------------------------------------------------------
// Upstream: Firebase
// ---------------------------------------------------------------------------

/// A single HN item (story / comment / job / poll / pollopt). The Firebase API
/// returns `null` for a missing item — callers deserialize into `Option<Item>`.
#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub by: Option<String>,
    pub time: Option<i64>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub score: Option<i64>,
    /// Total comment count (stories/polls).
    pub descendants: Option<i64>,
    pub kids: Option<Vec<u64>>,
    /// Parent item (comments/pollopts). Modeled per the HN schema; not surfaced
    /// in tool output today, but kept so the data model stays complete.
    #[allow(dead_code)]
    pub parent: Option<u64>,
    #[serde(default)]
    pub dead: bool,
    #[serde(default)]
    pub deleted: bool,
}

/// A HN user profile from `/user/{id}.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: String,
    pub created: Option<i64>,
    pub karma: Option<i64>,
    pub about: Option<String>,
    pub submitted: Option<Vec<u64>>,
}

// ---------------------------------------------------------------------------
// Upstream: Algolia
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct AlgoliaResponse {
    #[serde(default)]
    pub hits: Vec<AlgoliaHit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlgoliaHit {
    #[serde(rename = "objectID")]
    pub object_id: String,
    pub title: Option<String>,
    /// Present on comment hits; used as a title fallback.
    pub story_title: Option<String>,
    pub author: Option<String>,
    pub points: Option<i64>,
    pub num_comments: Option<i64>,
    pub url: Option<String>,
    pub story_url: Option<String>,
    pub created_at_i: Option<i64>,
}

// ---------------------------------------------------------------------------
// Tool output shapes (compact, model-friendly)
// ---------------------------------------------------------------------------

/// One row of a ranked story list. No comment trees here — that's `get_item`.
#[derive(Debug, Serialize)]
pub struct StorySummary {
    pub id: u64,
    pub title: Option<String>,
    pub by: Option<String>,
    pub score: Option<i64>,
    /// Comment count.
    pub descendants: Option<i64>,
    pub url: Option<String>,
    pub hn_url: String,
}

impl StorySummary {
    pub fn from_item(item: Item) -> Self {
        let hn_url = hn_item_url(item.id);
        Self {
            id: item.id,
            title: item.title,
            by: item.by,
            score: item.score,
            descendants: item.descendants,
            url: item.url,
            hn_url,
        }
    }
}

/// A node in the trimmed comment tree returned by `get_item`.
#[derive(Debug, Serialize)]
pub struct CommentNode {
    pub id: u64,
    pub by: Option<String>,
    pub text: Option<String>,
    pub time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_iso: Option<String>,
    pub depth: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<CommentNode>,
}

/// Full detail for a single item, optionally with a capped comment tree.
#[derive(Debug, Serialize)]
pub struct ItemDetail {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub by: Option<String>,
    pub time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_iso: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub hn_url: String,
    pub score: Option<i64>,
    pub descendants: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<CommentNode>>,
    /// e.g. "12 more comments not shown" when the tree was capped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation_note: Option<String>,
}

impl ItemDetail {
    pub fn from_item(item: Item) -> Self {
        let hn_url = hn_item_url(item.id);
        let time_iso = item.time.and_then(unix_to_iso);
        Self {
            id: item.id,
            kind: item.kind,
            by: item.by,
            time: item.time,
            time_iso,
            title: item.title,
            text: item.text,
            url: item.url,
            hn_url,
            score: item.score,
            descendants: item.descendants,
            comments: None,
            truncation_note: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UserProfile {
    pub id: String,
    pub karma: Option<i64>,
    pub about: Option<String>,
    pub created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_iso: Option<String>,
    pub submitted_count: usize,
}

impl UserProfile {
    pub fn from_user(user: User) -> Self {
        let created_iso = user.created.and_then(unix_to_iso);
        let submitted_count = user.submitted.as_ref().map_or(0, Vec::len);
        Self {
            id: user.id,
            karma: user.karma,
            about: user.about,
            created: user.created,
            created_iso,
            submitted_count,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub id: Option<u64>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub points: Option<i64>,
    pub num_comments: Option<i64>,
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at_iso: Option<String>,
    pub url: Option<String>,
    pub hn_url: Option<String>,
}

impl SearchHit {
    pub fn from_hit(hit: AlgoliaHit) -> Self {
        let id = hit.object_id.parse::<u64>().ok();
        let hn_url = id.map(hn_item_url);
        let created_at_iso = hit.created_at_i.and_then(unix_to_iso);
        Self {
            id,
            title: hit.title.or(hit.story_title),
            author: hit.author,
            points: hit.points,
            num_comments: hit.num_comments,
            created_at: hit.created_at_i,
            created_at_iso,
            url: hit.url.or(hit.story_url),
            hn_url,
        }
    }
}
