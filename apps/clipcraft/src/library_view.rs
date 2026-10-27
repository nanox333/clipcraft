//! Derived library metadata and ordered indices, rebuilt only when the model/view changes.
use crate::organize::LibraryFilter;
use clipcraft_engine::{GameGroup, LibraryScanResult, game_of};
use std::collections::HashMap;

/// How the clips grid is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Sort {
    #[default]
    Name,
    NameReverse,
    Newest,
    Oldest,
    Largest,
    Smallest,
    Game,
}

impl Sort {
    pub(crate) const ALL: [Sort; 7] = [
        Sort::Newest,
        Sort::Oldest,
        Sort::Name,
        Sort::NameReverse,
        Sort::Largest,
        Sort::Smallest,
        Sort::Game,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Sort::Name => "Name A–Z",
            Sort::NameReverse => "Name Z–A",
            Sort::Newest => "Newest first",
            Sort::Oldest => "Oldest first",
            Sort::Largest => "Largest first",
            Sort::Smallest => "Smallest first",
            Sort::Game => "Game A–Z",
        }
    }
}

impl From<bool> for Sort {
    /// The old two-way choice: `true` is newest first.
    fn from(newest: bool) -> Self {
        if newest { Sort::Newest } else { Sort::Name }
    }
}

#[derive(Default)]
struct Row {
    name: String,
    game: Option<String>,
    game_key: Option<String>,
}
#[derive(Default)]
pub(crate) struct ViewCache {
    ready: bool,
    directory: String,
    rows: Vec<Row>,
    names: Vec<usize>,
    orders: HashMap<Sort, Vec<usize>>,
    key: Option<(LibraryFilter, String, Sort)>,
    pub indices: Vec<usize>,
    pub total: usize,
    pub favorites: usize,
    pub collections: HashMap<String, usize>,
    pub games: Vec<GameGroup>,
    #[cfg(test)]
    builds: usize,
}
impl ViewCache {
    pub fn invalidate(&mut self) {
        self.ready = false;
        self.key = None;
    }
    pub fn game(&self, index: usize) -> Option<&str> {
        self.rows.get(index)?.game.as_deref()
    }
    pub fn prepare(
        &mut self,
        result: &LibraryScanResult,
        filter: &LibraryFilter,
        query: &str,
        sort: impl Into<Sort>,
    ) {
        let sort = sort.into();
        if !self.ready
            || self.directory != result.directory
            || self.rows.len() != result.entries.len()
        {
            self.rebuild(result);
        }
        let key = (filter.clone(), query.trim().to_lowercase(), sort);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let game_key = match filter {
            LibraryFilter::Game(name) => Some(name.to_lowercase()),
            _ => None,
        };
        self.indices.clear();
        self.total = 0;
        let order = self.orders.get(&sort).unwrap_or(&self.names);
        for &index in order {
            let (Some(entry), Some(row)) = (result.entries.get(index), self.rows.get(index)) else {
                continue;
            };
            let matches = match filter {
                LibraryFilter::All => true,
                LibraryFilter::Favorites => entry.favorite,
                LibraryFilter::Collection(name) => entry.collections.contains(name),
                LibraryFilter::Game(_) => row.game_key == game_key,
            };
            if matches {
                self.total += 1;
                if key.1.is_empty() || row.name.contains(&key.1) {
                    self.indices.push(index);
                }
            }
        }
        self.key = Some(key);
    }
    fn rebuild(&mut self, result: &LibraryScanResult) {
        self.directory.clone_from(&result.directory);
        self.rows.clear();
        self.favorites = 0;
        self.collections.clear();
        self.games.clear();
        let mut groups = HashMap::<String, usize>::new();
        for entry in &result.entries {
            self.favorites += usize::from(entry.favorite);
            for name in &entry.collections {
                *self.collections.entry(name.clone()).or_default() += 1;
            }
            let game = game_of(entry, &result.directory);
            let game_key = game.as_ref().map(|g| g.to_lowercase());
            if let (Some(name), Some(key)) = (&game, &game_key) {
                if let Some(&index) = groups.get(key) {
                    self.games[index].clips += 1;
                } else {
                    groups.insert(key.clone(), self.games.len());
                    self.games.push(GameGroup {
                        name: name.clone(),
                        clips: 1,
                    });
                }
            }
            self.rows.push(Row {
                name: entry.title.to_lowercase(),
                game,
                game_key,
            });
        }
        self.games
            .sort_by_cached_key(|g| (std::cmp::Reverse(g.clips), g.name.to_lowercase()));
        self.names = (0..self.rows.len()).collect();
        self.names.sort_by(|&a, &b| {
            self.rows[a]
                .name
                .cmp(&self.rows[b].name)
                .then_with(|| result.entries[a].path.cmp(&result.entries[b].path))
        });
        // Stable sorts keep the name/path order for ties without normalizing names per comparison.
        let entries = &result.entries;
        let rows = &self.rows;
        let by = |sort: Sort, compare: &dyn Fn(usize, usize) -> std::cmp::Ordering| {
            let mut order = self.names.clone();
            order.sort_by(|&a, &b| compare(a, b));
            (sort, order)
        };
        let game = |i: usize| (rows[i].game_key.is_none(), rows[i].game_key.clone());
        let orders = [
            by(Sort::Newest, &|a, b| {
                entries[b]
                    .modified_unix_seconds
                    .cmp(&entries[a].modified_unix_seconds)
            }),
            by(Sort::Oldest, &|a, b| {
                entries[a]
                    .modified_unix_seconds
                    .cmp(&entries[b].modified_unix_seconds)
            }),
            by(Sort::NameReverse, &|a, b| rows[b].name.cmp(&rows[a].name)),
            by(Sort::Largest, &|a, b| {
                entries[b].size_bytes.cmp(&entries[a].size_bytes)
            }),
            by(Sort::Smallest, &|a, b| {
                entries[a].size_bytes.cmp(&entries[b].size_bytes)
            }),
            by(Sort::Game, &|a, b| game(a).cmp(&game(b))),
        ];
        self.orders = orders.into_iter().collect();
        self.ready = true;
        self.key = None;
        #[cfg(test)]
        {
            self.builds += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_sort_order_puts_the_right_clip_first() {
        let mut result = LibraryScanResult {
            directory: "/clips".into(),
            entries: Vec::new(),
            persisted: false,
            collections: Vec::new(),
        };
        for (name, size, modified, game) in [
            ("b.mp4", 20, 200, Some("Zelda")),
            ("a.mp4", 30, 300, Some("Doom")),
            ("c.mp4", 10, 100, None),
        ] {
            let mut entry = crate::tests::media_entry(name, Some(modified));
            entry.size_bytes = size;
            entry.game = game.map(str::to_owned);
            result.entries.push(entry);
        }
        let mut cache = ViewCache::default();
        let first = |cache: &mut ViewCache, sort: Sort| {
            cache.prepare(&result, &LibraryFilter::All, "", sort);
            result.entries[cache.indices[0]].title.clone()
        };
        assert_eq!(first(&mut cache, Sort::Name), "a.mp4");
        assert_eq!(first(&mut cache, Sort::NameReverse), "c.mp4");
        assert_eq!(first(&mut cache, Sort::Newest), "a.mp4");
        assert_eq!(first(&mut cache, Sort::Oldest), "c.mp4");
        assert_eq!(first(&mut cache, Sort::Largest), "a.mp4");
        assert_eq!(first(&mut cache, Sort::Smallest), "c.mp4");
        // Clips with no game sort last.
        assert_eq!(first(&mut cache, Sort::Game), "a.mp4");
        cache.prepare(&result, &LibraryFilter::All, "", Sort::Game);
        assert_eq!(
            result.entries[*cache.indices.last().unwrap()].title,
            "c.mp4"
        );
    }

    #[test]
    fn cached_views_counts_and_groups_match_reference_and_invalidate() {
        let mut result = LibraryScanResult {
            directory: "/clips".into(),
            entries: Vec::new(),
            persisted: false,
            collections: vec!["Best".into()],
        };
        for i in 0..10_000 {
            let mut entry = crate::tests::media_entry(
                &format!("Round {} — GAME.mp4", (i * 7919) % 10_000),
                if i % 9 == 0 { None } else { Some(i % 97) },
            );
            entry.favorite = i % 3 == 0;
            if i % 7 == 0 {
                entry.collections.push("Best".into());
            }
            entry.game = Some(if i % 2 == 0 { "Hades" } else { "Celeste" }.into());
            result.entries.push(entry);
        }
        let mut cache = ViewCache::default();
        for filter in [
            LibraryFilter::All,
            LibraryFilter::Favorites,
            LibraryFilter::Collection("Best".into()),
            LibraryFilter::Game("HADES".into()),
        ] {
            for query in ["", "round 12", "— game", "missing"] {
                for newest in [false, true] {
                    cache.prepare(&result, &filter, query, newest);
                    let expected = crate::filtered_entries(
                        &result.entries,
                        &filter,
                        &result.directory,
                        query,
                        newest,
                    );
                    let paths: Vec<_> = cache
                        .indices
                        .iter()
                        .map(|&i| &result.entries[i].path)
                        .collect();
                    assert_eq!(paths, expected.iter().map(|e| &e.path).collect::<Vec<_>>());
                    assert_eq!(
                        cache.total,
                        result
                            .entries
                            .iter()
                            .filter(|e| filter.matches(e, &result.directory))
                            .count()
                    );
                }
            }
        }
        assert_eq!(cache.builds, 1);
        assert_eq!(
            cache.games,
            clipcraft_engine::games(&result.entries, &result.directory)
        );
        cache.prepare(&result, &LibraryFilter::All, "", false);
        let pointer = cache.indices.as_ptr();
        cache.prepare(&result, &LibraryFilter::All, "", false);
        assert_eq!(pointer, cache.indices.as_ptr());
        result.entries[0].title = "AAA renamed".into();
        result.entries[0].game = Some("Changed".into());
        cache.invalidate();
        cache.prepare(&result, &LibraryFilter::All, "AAA", false);
        assert_eq!(cache.indices, [0]);
        assert_eq!(cache.game(0), Some("Changed"));
        assert_eq!(cache.builds, 2);
    }
}

/// The editor's already virtualized browser keeps insertion order and only caches search indices.
#[derive(Default)]
pub(crate) struct BrowserSearch {
    source: std::sync::Weak<Vec<clipcraft_engine::MediaEntry>>,
    names: Vec<String>,
    query: Option<String>,
    indices: std::sync::Arc<Vec<usize>>,
}
impl BrowserSearch {
    pub fn prepare(
        &mut self,
        media: &std::sync::Arc<Vec<clipcraft_engine::MediaEntry>>,
        query: &str,
    ) -> std::sync::Arc<Vec<usize>> {
        let source = std::sync::Arc::downgrade(media);
        if !self.source.ptr_eq(&source) {
            self.names = media.iter().map(|e| e.title.to_lowercase()).collect();
            self.source = source;
            self.query = None;
        }
        let query = query.to_lowercase();
        if self.query.as_ref() != Some(&query) {
            self.indices = std::sync::Arc::new(
                self.names
                    .iter()
                    .enumerate()
                    .filter_map(|(i, name)| name.contains(&query).then_some(i))
                    .collect(),
            );
            self.query = Some(query);
        }
        self.indices.clone()
    }
}
#[cfg(test)]
mod browser_tests {
    use super::*;
    #[test]
    fn editor_search_keeps_order_reuses_indices_and_observes_arc_mutation() {
        let mut media = std::sync::Arc::new(vec![
            crate::tests::media_entry("Zebra.mp4", None),
            crate::tests::media_entry("Äther.mp4", None),
        ]);
        let mut search = BrowserSearch::default();
        let first = search.prepare(&media, "");
        assert_eq!(*first, [0, 1]);
        assert!(std::sync::Arc::ptr_eq(&first, &search.prepare(&media, "")));
        assert_eq!(*search.prepare(&media, "ÄT"), [1]);
        std::sync::Arc::make_mut(&mut media)[0].title = "ÄTHER first.mp4".into();
        assert_eq!(*search.prepare(&media, "ät"), [0, 1]);
        assert!(search.prepare(&media, "missing").is_empty());
    }
}
