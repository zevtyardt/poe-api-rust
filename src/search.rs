use std::{collections::VecDeque, task::Poll};

use futures_util::{FutureExt, Stream};
use serde_json::{json, Value};

use crate::{
    api::PoeApi,
    bot::BotInfo,
    models::{query::QueryHash, user::UserInfo, EntityType, SearchData},
    queries::RequestData,
};

#[derive(Debug)]
pub enum Entity {
    User(UserInfo),
    Bot(BotInfo),
}

#[derive(Debug)]
pub struct SearchResult<'a> {
    api: &'a mut PoeApi,
    search_data: SearchData<'a>,
    cursor: Option<String>,
    results: VecDeque<Entity>,
    is_completed: bool,
}

impl Stream for SearchResult<'_> {
    type Item = Entity;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut fut = Box::pin(self.next_item());

        loop {
            match fut.poll_unpin(cx) {
                Poll::Ready(Ok(item)) => return Poll::Ready(item),
                Poll::Ready(Err(_)) => return Poll::Ready(None),
                _ => {}
            }
        }
    }
}

impl<'a> SearchResult<'a> {
    pub fn new(api: &'a mut PoeApi, search_data: SearchData<'a>) -> Self {
        Self {
            api,
            search_data,
            cursor: None,
            results: VecDeque::new(),
            is_completed: false,
        }
    }

    async fn next_item(&mut self) -> anyhow::Result<Option<Entity>> {
        while !self.is_completed || !self.results.is_empty() {
            if let Some(bot_info) = self.results.pop_front() {
                return Ok(Some(bot_info));
            }

            if self.search_data.entity_type == EntityType::User && self.search_data.query.is_none()
            {
                self.search_data.query = Some("");
            }

            let (response, connection_type) = if let Some(query) = self.search_data.query {
                let mut data = json!({
                    "query": query,
                    "entityType": self.search_data.entity_type,
                    "count": 50
                });
                if let (Some(cursor), Some(object)) = (self.cursor.clone(), data.as_object_mut()) {
                    object.insert("cursor".into(), Value::String(cursor));
                }

                let response = self
                    .api
                    .send_request(RequestData {
                        query_name: QueryHash::SearchResultsListPaginationQuery,
                        data,
                        ..Default::default()
                    })
                    .await?;
                (response, "searchEntityConnection")
            } else {
                let mut data = json!({
                    "categoryName": self.search_data.category_name,
                    "count": self.search_data.count

                });
                if let (Some(cursor), Some(object)) = (self.cursor.clone(), data.as_object_mut()) {
                    object.insert("cursor".into(), Value::String(cursor));
                }

                let response = self
                    .api
                    .send_request(RequestData {
                        query_name: QueryHash::ExploreBotsListPaginationQuery,
                        data,
                        ..Default::default()
                    })
                    .await?;
                (response, "exploreBotsConnection")
            };

            self.is_completed = true;
            if let Some(data) = response.get("data").and_then(|d| d.get(connection_type)) {
                self.cursor = data
                    .get("pageInfo")
                    .and_then(|p| p.get("endCursor"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());
                if let Some(items) = data.get("edges").and_then(|v| v.as_array()) {
                    for item in items {
                        if let Some(node) = item.get("node") {
                            if self.search_data.entity_type == EntityType::Bot {
                                let bot_info = serde_json::from_value::<BotInfo>(node.clone())?;
                                self.results.push_back(Entity::Bot(bot_info));
                                self.is_completed = false;
                            } else {
                                let user_info = serde_json::from_value::<UserInfo>(node.clone())?;
                                self.results.push_back(Entity::User(user_info));
                                self.is_completed = false;
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }
}
