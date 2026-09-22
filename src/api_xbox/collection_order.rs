//! Wire-format parsers shared with host tests. Never sort these responses.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentPage {
    pub results: Vec<RecentTitle>,
    pub continuation_token: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentTitle {
    pub title_id: String,
}

pub(crate) fn recent_page(value: serde_json::Value) -> Result<RecentPage> {
    serde_json::from_value(value).context("invalid Xbox recent-games response")
}

pub(crate) fn gallery_products(value: serde_json::Value) -> Result<Vec<String>> {
    let entries = value.as_array().context("invalid Xbox gallery response")?;
    let mut ids = Vec::new();
    for entry in entries {
        if let Some(id) = entry.get("id").and_then(|id| id.as_str()) {
            ensure!(!id.is_empty(), "empty Xbox product id");
            ids.push(id.to_owned());
        } else {
            // SIGL starts with a metadata row; unknown rows are not an empty list.
            ensure!(entry.get("siglId").is_some(), "invalid Xbox gallery entry");
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn history_keeps_server_order_and_continuation() {
        let page=recent_page(json!({"results":[{"titleId":"z"},{"titleId":"a"}],"continuationToken":"next+/="})).unwrap();
        assert_eq!(page.results.iter().map(|r|r.title_id.as_str()).collect::<Vec<_>>(),["z","a"]);
        assert_eq!(page.continuation_token.as_deref(),Some("next+/="));
        assert!(recent_page(json!({"error":"expired"})).is_err());
        assert!(recent_page(json!({"results":[]})).unwrap().results.is_empty());
    }
    #[test]
    fn gallery_order_is_not_release_date_or_alphabetical_order() {
        assert_eq!(gallery_products(json!([{"siglId":"feed","title":"New"},{"id":"Z"},{"id":"A"}])).unwrap(),["Z","A"]);
        assert!(gallery_products(json!({"error":"unavailable"})).is_err());
        assert!(gallery_products(json!([{"unexpected":true}])).is_err());
    }
}
