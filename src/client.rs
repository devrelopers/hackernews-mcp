//! HTTP layer over the two public Hacker News APIs.
//!
//! A single [`reqwest::Client`] is built once and reused for every request so
//! connections are pooled. No auth, no keys — both APIs are public.

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};

use crate::types::{AlgoliaResponse, Item, User};

const FIREBASE_BASE: &str = "https://hacker-news.firebaseio.com/v0";
const ALGOLIA_BASE: &str = "https://hn.algolia.com/api/v1";
const USER_AGENT: &str = "hackernews-mcp/0.1";

/// Max in-flight item fetches. Kept modest to be polite to the Firebase API.
const CONCURRENCY: usize = 10;

/// The story feed categories exposed by the Firebase API.
#[derive(Debug, Clone, Copy)]
pub enum Category {
    Top,
    New,
    Best,
    Ask,
    Show,
    Job,
}

impl Category {
    /// The Firebase endpoint path segment for this feed.
    fn endpoint(self) -> &'static str {
        match self {
            Category::Top => "topstories",
            Category::New => "newstories",
            Category::Best => "beststories",
            Category::Ask => "askstories",
            Category::Show => "showstories",
            Category::Job => "jobstories",
        }
    }
}

/// How Algolia should rank results.
#[derive(Debug, Clone, Copy)]
pub enum SearchSort {
    /// `/search` — relevance ranked.
    Relevance,
    /// `/search_by_date` — newest first.
    Date,
}

#[derive(Clone)]
pub struct HnClient {
    http: reqwest::Client,
}

impl HnClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { http })
    }

    /// Fetch the ordered ID list for a story feed.
    pub async fn story_ids(&self, category: Category) -> Result<Vec<u64>> {
        let url = format!("{FIREBASE_BASE}/{}.json", category.endpoint());
        let ids = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?
            .error_for_status()
            .with_context(|| format!("error status from {url}"))?
            .json::<Vec<u64>>()
            .await
            .with_context(|| format!("decoding story IDs from {url}"))?;
        Ok(ids)
    }

    /// Fetch a single item. Returns `Ok(None)` when the API responds with
    /// `null` (deleted item or invalid ID) rather than erroring.
    pub async fn item(&self, id: u64) -> Result<Option<Item>> {
        let url = format!("{FIREBASE_BASE}/item/{id}.json");
        let item = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("requesting item {id}"))?
            .error_for_status()
            .with_context(|| format!("error status fetching item {id}"))?
            .json::<Option<Item>>()
            .await
            .with_context(|| format!("decoding item {id}"))?;
        Ok(item)
    }

    /// Fetch a user profile. Returns `Ok(None)` for an unknown user.
    pub async fn user(&self, id: &str) -> Result<Option<User>> {
        let url = format!("{FIREBASE_BASE}/user/{id}.json");
        let user = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("requesting user {id}"))?
            .error_for_status()
            .with_context(|| format!("error status fetching user {id}"))?
            .json::<Option<User>>()
            .await
            .with_context(|| format!("decoding user {id}"))?;
        Ok(user)
    }

    /// Fetch many items concurrently (capped at [`CONCURRENCY`]), preserving the
    /// order of `ids`. Items that fail to fetch or come back `null` are dropped.
    pub async fn items_in_order(&self, ids: &[u64]) -> Vec<Item> {
        let mut indexed: Vec<(usize, Item)> = stream::iter(ids.iter().copied().enumerate())
            .map(|(idx, id)| async move { self.item(id).await.ok().flatten().map(|it| (idx, it)) })
            .buffer_unordered(CONCURRENCY)
            .filter_map(|res| async move { res })
            .collect()
            .await;
        indexed.sort_by_key(|(idx, _)| *idx);
        indexed.into_iter().map(|(_, item)| item).collect()
    }

    /// Full-text search via Algolia.
    pub async fn search(
        &self,
        query: &str,
        sort: SearchSort,
        tags: Option<&str>,
        numeric_filters: Option<&str>,
        hits_per_page: u32,
    ) -> Result<AlgoliaResponse> {
        let path = match sort {
            SearchSort::Relevance => "search",
            SearchSort::Date => "search_by_date",
        };
        let url = format!("{ALGOLIA_BASE}/{path}");
        let hits_per_page = hits_per_page.to_string();

        let mut params: Vec<(&str, &str)> =
            vec![("query", query), ("hitsPerPage", &hits_per_page)];
        if let Some(tags) = tags {
            params.push(("tags", tags));
        }
        if let Some(filters) = numeric_filters {
            params.push(("numericFilters", filters));
        }

        let resp = self
            .http
            .get(&url)
            .query(&params)
            .send()
            .await
            .with_context(|| format!("requesting Algolia search for {query:?}"))?
            .error_for_status()
            .context("error status from Algolia search")?
            .json::<AlgoliaResponse>()
            .await
            .context("decoding Algolia search response")?;
        Ok(resp)
    }
}
