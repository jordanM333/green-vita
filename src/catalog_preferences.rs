//! Local catalog organization, keyed by provider-stable game ID, never row index.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogSection {
    #[default]
    All,
    Favorites,
    RecentlyPlayed,
    RecentlyAdded,
}
impl CatalogSection {
    pub const ALL: [Self; 4] = [Self::All, Self::Favorites, Self::RecentlyPlayed, Self::RecentlyAdded];
    pub fn label_key(self) -> &'static str {
        match self {
            Self::All => "catalog-all",
            Self::Favorites => "catalog-favorites",
            Self::RecentlyPlayed => "catalog-recently-played",
            Self::RecentlyAdded => "catalog-recently-added",
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
    pub recently_added: Option<Vec<String>>,
}
impl CatalogCollections {
    pub fn supports(&self, section: CatalogSection) -> bool {
        section != CatalogSection::RecentlyAdded || self.recently_added.is_some()
    }
    pub fn indices(&self, ids: &[&str], section: CatalogSection, prefs: &CatalogPreferences) -> Vec<usize> {
        match section {
            CatalogSection::All => (0..ids.len()).collect(),
            CatalogSection::Favorites => ids.iter().enumerate()
                .filter_map(|(i,id)| prefs.favorites.contains(*id).then_some(i)).collect(),
            CatalogSection::RecentlyPlayed => ordered_indices(ids, &prefs.recently_played),
            CatalogSection::RecentlyAdded => self.recently_added.as_ref()
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
    fn favorites_survive_catalog_reorder_and_temporarily_missing_games() {
        let mut prefs=CatalogPreferences::default(); prefs.toggle_favorite("b");
        let collections=CatalogCollections::default();
        assert_eq!(collections.indices(&["a","b"],CatalogSection::Favorites,&prefs),vec![1]);
        assert!(collections.indices(&["a"],CatalogSection::Favorites,&prefs).is_empty());
        assert_eq!(collections.indices(&["b","a"],CatalogSection::Favorites,&prefs),vec![0]);
        prefs.toggle_favorite("b"); assert!(prefs.favorites.is_empty());
    }
    #[test]
    fn replay_moves_game_to_front_without_duplicates_and_bounds_history() {
        let mut prefs=CatalogPreferences::default();
        for n in 0..60 {prefs.record_played(&n.to_string());}
        prefs.record_played("55"); assert_eq!(prefs.recently_played.len(),50);
        let roundtrip: CatalogPreferences=serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(CatalogCollections::default().indices(&["58","55","59"],CatalogSection::RecentlyPlayed,&roundtrip),vec![1,2,0]);
    }
    #[test]
    fn added_collection_uses_provider_order_deduplicates_and_filters_unavailable_ids() {
        let mut collections=CatalogCollections::default();
        assert!(!collections.supports(CatalogSection::RecentlyAdded));
        collections.recently_added=Some(vec!["missing".into(),"b".into(),"b".into(),"a".into()]);
        assert!(collections.supports(CatalogSection::RecentlyAdded));
        assert_eq!(collections.indices(&["a","b"],CatalogSection::RecentlyAdded,&CatalogPreferences::default()),vec![1,0]);
    }
}
