//! Provider-ordered collections, keyed by stable game ID, never row index.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogSection {
    #[default]
    All,
    Favorites,
    RecentlyPlayed,
    RecentlyAdded,
    MostPopular,
}
impl CatalogSection {
    pub const ALL: [Self; 5] = [Self::All, Self::RecentlyPlayed, Self::RecentlyAdded, Self::MostPopular, Self::Favorites];
    pub fn label_key(self) -> &'static str {
        match self {
            Self::All => "catalog-all",
            Self::Favorites => "catalog-favorites",
            Self::RecentlyPlayed => "catalog-recently-played",
            Self::RecentlyAdded => "catalog-recently-added",
            Self::MostPopular => "catalog-most-popular",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogPreferences {
    pub favorites: BTreeSet<String>,
    /// Most recently successfully started first. This is local, not Xbox history.
    pub recently_played: Vec<String>,
}
impl CatalogPreferences {
    pub fn toggle_favorite(&mut self, id: &str) {
        if !self.favorites.remove(id) { self.favorites.insert(id.to_owned()); }
    }
    pub fn record_played(&mut self, id: &str) {
        self.recently_played.retain(|saved| saved != id);
        self.recently_played.insert(0, id.to_owned());
        self.recently_played.truncate(50);
    }
}

/// Provider-supplied order in stable game IDs. None means unsupported/not loaded;
/// Some(empty) means loaded with no matching titles. Never derive this from a
/// game's release date or from the order in which its metadata finished loading.
#[derive(Default)]
pub struct CatalogCollections {
    pub recently_played: Option<Vec<String>>,
    pub recently_added: Option<Vec<String>>,
    pub most_popular: Option<Vec<String>>,
    pub errors: Vec<CatalogSection>,
}
impl CatalogCollections {
    pub fn supports(&self, section: CatalogSection) -> bool {
        // Account favorites need a verified service contract. Local bookmarks
        // remain in old settings for migration, but are not presented as synced.
        section != CatalogSection::Favorites
    }
    pub fn indices(&self, ids: &[&str], section: CatalogSection, _prefs: &CatalogPreferences) -> Vec<usize> {
        match section {
            CatalogSection::All => (0..ids.len()).collect(),
            CatalogSection::Favorites => Vec::new(),
            CatalogSection::RecentlyPlayed => self.recently_played.as_ref()
                .map(|order| ordered_indices(ids, order)).unwrap_or_default(),
            CatalogSection::RecentlyAdded => self.recently_added.as_ref()
                .map(|order| ordered_indices(ids, order)).unwrap_or_default(),
            CatalogSection::MostPopular => self.most_popular.as_ref()
                .map(|order| ordered_indices(ids, order)).unwrap_or_default(),
        }
    }
}
fn ordered_indices(ids: &[&str], order: &[String]) -> Vec<usize> {
    let lookup: HashMap<_,_> = ids.iter().enumerate().map(|(i,id)|(*id,i)).collect();
    let mut seen = HashSet::new();
    order.iter().filter_map(|id| lookup.get(id.as_str()).copied())
        .filter(|i| seen.insert(*i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_favorites_are_not_presented_as_account_favorites() {
        let mut prefs=CatalogPreferences::default(); prefs.toggle_favorite("b");
        let collections=CatalogCollections::default();
        assert!(!collections.supports(CatalogSection::Favorites));
        assert!(collections.indices(&["a","b"],CatalogSection::Favorites,&prefs).is_empty());
        prefs.toggle_favorite("b"); assert!(prefs.favorites.is_empty());
    }
    #[test]
    fn replay_moves_game_to_front_without_duplicates_and_bounds_history() {
        let mut prefs=CatalogPreferences::default();
        for n in 0..60 {prefs.record_played(&n.to_string());}
        prefs.record_played("55"); assert_eq!(prefs.recently_played.len(),50);
        let roundtrip: CatalogPreferences=serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert!(CatalogCollections::default().indices(&["58","55","59"],CatalogSection::RecentlyPlayed,&roundtrip).is_empty());
        let collections=CatalogCollections { recently_played:Some(vec!["59".into(),"55".into()]), ..Default::default() };
        assert_eq!(collections.indices(&["58","55","59"],CatalogSection::RecentlyPlayed,&roundtrip),vec![2,1]);
    }
    #[test]
    fn added_collection_uses_provider_order_deduplicates_and_filters_unavailable_ids() {
        let mut collections=CatalogCollections::default();
        assert!(collections.recently_added.is_none());
        collections.recently_added=Some(vec!["missing".into(),"b".into(),"b".into(),"a".into()]);
        assert!(collections.supports(CatalogSection::RecentlyAdded));
        assert_eq!(collections.indices(&["a","b"],CatalogSection::RecentlyAdded,&CatalogPreferences::default()),vec![1,0]);
    }
}
