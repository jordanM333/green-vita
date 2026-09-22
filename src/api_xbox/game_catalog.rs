//! Xbox-specific adapter for the provider-neutral game catalog.

use crate::api::catalog::{Game, GameDetails};
use crate::api_xbox::api::ApiClient;
use crate::api_xbox::catalog;
use anyhow::Result;
use reqwest::Client;

pub(crate) async fn load_collections(api: &ApiClient, games: &[(String, Option<String>)], market: &str, language: &str)
    -> crate::catalog_preferences::CatalogCollections
{
    use crate::catalog_preferences::{CatalogCollections, CatalogSection};
    use super::collection_order::{recent_page, gallery_products};
    // IDs observed in xbox.com/play's current client bundle. Keep provider order.
    let recent = async {
        let mut order = Vec::new();
        let mut continuation = None;
        let mut tokens = std::collections::HashSet::new();
        for _ in 0..20 {
            let page = recent_page(api.get_recent_titles(continuation.as_deref()).await?)?;
            order.extend(page.results.into_iter().map(|entry| entry.title_id));
            continuation = page.continuation_token.filter(|token| !token.is_empty());
            match continuation.as_ref() {
                None => return Ok::<_, anyhow::Error>(order),
                Some(token) => anyhow::ensure!(tokens.insert(token.clone()), "repeated history continuation token"),
            }
        }
        anyhow::bail!("Xbox history exceeded pagination limit")
    };
    let gallery = |id| async move {
        let products = gallery_products(api.get_gallery(id, market, language).await?)?;
        let lookup: std::collections::HashMap<_,_> = games.iter()
            .filter_map(|(id, product)| Some((product.as_deref()?, id.as_str()))).collect();
        Ok::<_, anyhow::Error>(products.iter().filter_map(|id| lookup.get(id.as_str()).map(|id| (*id).to_owned())).collect::<Vec<_>>())
    };
    let (recent, added, popular) = tokio::join!(recent,
        gallery("06323672-b8c8-43cc-b0de-32d5a9834749"),
        gallery("6a589fa0-d493-472b-8e20-3813699d7056"));
    let mut result = CatalogCollections::default();
    for (section, response) in [(CatalogSection::RecentlyPlayed, recent), (CatalogSection::RecentlyAdded, added), (CatalogSection::MostPopular, popular)] {
        match response {
            Ok(ids) => match section {
                CatalogSection::RecentlyPlayed => result.recently_played = Some(ids),
                CatalogSection::RecentlyAdded => result.recently_added = Some(ids),
                CatalogSection::MostPopular => result.most_popular = Some(ids),
                _ => unreachable!(),
            },
            Err(_) => { result.errors.push(section); } // No account tokens or response bodies in logs.
        }
    }
    result
}

pub async fn load_games(api: &ApiClient) -> Result<Vec<Game>> {
    let response = api.get_titles().await?;
    Ok(extract_games(&response))
}

pub async fn fetch_details(
    client: &Client,
    metadata_id: &str,
    market: &str,
    language: &str,
) -> Result<GameDetails> {
    let details = catalog::fetch_title_details(client, metadata_id, market, language).await?;
    Ok(GameDetails {
        name: details.name,
        description: details.description,
        box_art_url: details.box_art_url,
        background_url: details.background_url,
        icon_url: details.icon_url,
        genres: details.genres,
        developer: details.developer,
        publisher: details.publisher,
        release_date: details.release_date,
        average_rating: details.average_rating,
        rating_count: details.rating_count,
        content_rating: details.content_rating,
    })
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TitlesResponse {
    #[serde(default)]
    results: Vec<TitleEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TitleEntry {
    #[serde(rename = "titleId")]
    slug: Option<String>,
    #[serde(default)]
    details: TitleDetails,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct TitleDetails {
    #[serde(rename = "productId")]
    product_id: Option<String>,
    #[serde(rename = "hasEntitlement", default)]
    has_entitlement: bool,
    #[serde(default)]
    programs: Vec<String>,
    #[serde(rename = "userSubscriptions", default)]
    user_subscriptions: Vec<String>,
}

fn extract_games(value: &serde_json::Value) -> Vec<Game> {
    let Ok(response) = serde_json::from_value::<TitlesResponse>(value.clone()) else {
        return Vec::new();
    };

    let mut games: Vec<Game> = response
        .results
        .into_iter()
        .filter_map(|entry| {
            let slug = entry.slug?;
            let is_playable = entry.details.has_entitlement
                || entry
                    .details
                    .programs
                    .iter()
                    .any(|program| entry.details.user_subscriptions.contains(program));
            if !is_playable {
                return None;
            }
            Some(Game {
                id: slug.clone(),
                launch_id: slug,
                metadata_id: entry.details.product_id,
                details: None,
                icon: None,
                box_art: None,
                background: None,
            })
        })
        .collect();

    games.sort_by(|left, right| left.id.cmp(&right.id));
    games.dedup_by(|left, right| left.id == right.id);
    games
}
