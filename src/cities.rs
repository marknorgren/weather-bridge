use crate::{Error, ErrorCode};
use rmcp::schemars;
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Most suggestions returned by `search` and by `CITY_NOT_FOUND`.
const SUGGESTION_LIMIT: usize = 10;
/// Fewest characters a city query may have after trimming. REST, CLI and MCP schemas
/// advertise this bound through `city_query_schema`, so change it here only.
pub const CITY_QUERY_MIN_CHARS: usize = 2;
/// Most characters a city query may have after trimming.
pub const CITY_QUERY_MAX_CHARS: usize = 120;
/// JSON Schema string lengths count raw input, so use a pattern for the trimmed bound.
/// This names Rust's Unicode White_Space set; ECMAScript \s differs for NEL and BOM.
/// Match a UTF-16 surrogate pair or a non-surrogate character so each scalar counts
/// once with or without ECMAScript Unicode mode.
pub(crate) fn city_query_schema(schema: &mut schemars::Schema) {
    const WHITESPACE: &str =
        r"\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000";
    const SCALAR: &str = r"(?:[\uD800-\uDBFF][\uDC00-\uDFFF]|[^\uD800-\uDFFF])";
    let non_whitespace_scalar =
        format!(r"(?:[\uD800-\uDBFF][\uDC00-\uDFFF]|[^\uD800-\uDFFF{WHITESPACE}])");
    let pattern = format!(
        r"^[{WHITESPACE}]*{non_whitespace_scalar}{SCALAR}{{{},{}}}{non_whitespace_scalar}[{WHITESPACE}]*(?![\s\S])",
        CITY_QUERY_MIN_CHARS - 2,
        CITY_QUERY_MAX_CHARS - 2,
    );
    schema.insert("pattern".into(), pattern.into());
}
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
#[schemars(description = "A city from the GeoNames-derived index (CC BY 4.0).")]
pub struct City {
    #[schemars(extend("description" = "GeoNames ID; use as cityId."))]
    pub id: u64,
    pub name: String,
    pub ascii_name: String,
    #[schemars(extend("description" = "Two-letter state or territory code."))]
    pub state: String,
    pub state_name: String,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
    pub population: u64,
    pub time_zone: String,
}
#[derive(Clone)]
struct IndexedCity {
    index: usize,
    point: [f64; 3],
}
impl RTreeObject for IndexedCity {
    type Envelope = AABB<[f64; 3]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.point)
    }
}
impl PointDistance for IndexedCity {
    fn distance_2(&self, point: &[f64; 3]) -> f64 {
        self.point
            .iter()
            .zip(point)
            .map(|(a, b)| (a - b).powi(2))
            .sum()
    }
}
fn sphere(lat: f64, lon: f64) -> [f64; 3] {
    let (lat, lon) = (lat.to_radians(), lon.to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}
/// Normalized search fields, computed once at load instead of on every search.
struct Keys {
    ascii_name: String,
    name: String,
    state: String,
    state_name: String,
    country: String,
}
#[derive(Clone)]
pub struct Cities {
    cities: Arc<Vec<City>>,
    keys: Arc<Vec<Keys>>,
    tree: Arc<RTree<IndexedCity>>,
    /// GeoNames ID to position in `cities`.
    by_id: Arc<HashMap<u64, usize>>,
}
pub fn normalize(s: &str) -> String {
    s.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
impl Cities {
    pub fn load() -> anyhow::Result<Self> {
        let cities: Vec<City> = serde_json::from_str(include_str!("../data/cities.json"))?;
        let keys = cities
            .iter()
            .map(|c| Keys {
                ascii_name: normalize(&c.ascii_name),
                name: normalize(&c.name),
                state: normalize(&c.state),
                state_name: normalize(&c.state_name),
                country: normalize(&c.country),
            })
            .collect();
        let tree = RTree::bulk_load(
            cities
                .iter()
                .enumerate()
                .map(|(index, c)| IndexedCity {
                    index,
                    point: sphere(c.latitude, c.longitude),
                })
                .collect(),
        );
        let mut by_id = HashMap::with_capacity(cities.len());
        for (index, c) in cities.iter().enumerate() {
            // Keep the first entry if an ID repeats, as a linear scan would.
            by_id.entry(c.id).or_insert(index);
        }
        Ok(Self {
            by_id: Arc::new(by_id),
            cities: Arc::new(cities),
            keys: Arc::new(keys),
            tree: Arc::new(tree),
        })
    }
    pub fn search(&self, query: &str) -> Result<Vec<City>, Error> {
        Ok(self
            .ranked(query)?
            .into_iter()
            .take(SUGGESTION_LIMIT)
            .map(|(_, c)| c.clone())
            .collect())
    }
    /// Every index entry matching `query`, exact matches (rank 0) before prefix matches,
    /// then by population descending and id.
    fn ranked(&self, query: &str) -> Result<Vec<(u8, &City)>, Error> {
        if !(CITY_QUERY_MIN_CHARS..=CITY_QUERY_MAX_CHARS).contains(&query.trim().chars().count()) {
            return Err(Error::invalid(format!(
                "Enter a city name between {CITY_QUERY_MIN_CHARS} and {CITY_QUERY_MAX_CHARS} characters, optionally followed by a state: Seattle, WA."
            )));
        }
        let pieces: Vec<String> = query.split(',').map(normalize).collect();
        if pieces.len() > 3 || pieces.iter().any(String::is_empty) {
            return Err(Error::invalid("Use City, State or City, State, US."));
        }
        let name = &pieces[0];
        let mut matches: Vec<(u8, &City)> = self
            .cities
            .iter()
            .zip(self.keys.iter())
            .filter_map(|(c, k)| {
                if pieces
                    .get(1)
                    .is_some_and(|state| state != &k.state && state != &k.state_name)
                {
                    return None;
                }
                // Territories keep their own country code, but all of them are US.
                if pieces
                    .get(2)
                    .is_some_and(|country| country != "us" && country != &k.country)
                {
                    return None;
                }
                let rank = if k.ascii_name == *name || k.name == *name {
                    0
                } else if k.ascii_name.starts_with(name.as_str()) {
                    1
                } else {
                    return None;
                };
                Some((rank, c))
            })
            .collect();
        matches.sort_by(|(ar, a), (br, b)| {
            ar.cmp(br)
                .then(b.population.cmp(&a.population))
                .then(a.id.cmp(&b.id))
        });
        Ok(matches)
    }
    /// Resolve an exact city name. Ambiguity lists every exact match in the index;
    /// not-found lists up to `SUGGESTION_LIMIT` prefix suggestions.
    pub fn resolve(&self, query: &str) -> Result<City, Error> {
        let ranked = self.ranked(query)?;
        let exact = ranked.iter().take_while(|(rank, _)| *rank == 0).count();
        match exact {
            1 => Ok(ranked[0].1.clone()),
            0 => Err(Error::new(
                ErrorCode::CityNotFound,
                "No exact city match. Choose a suggestion, add a state, or supply latitude and longitude. This index covers US cities and territories with population over 1,000 or administrative seats.",
            )
            .with_choices(
                ranked
                    .into_iter()
                    .take(SUGGESTION_LIMIT)
                    .map(|(_, c)| c.clone())
                    .collect(),
            )),
            _ => Err(Error::new(
                ErrorCode::AmbiguousCity,
                "Several cities share that name. Add the state or use coordinates from a choice.",
            )
            .with_choices(
                ranked
                    .into_iter()
                    .take(exact)
                    .map(|(_, c)| c.clone())
                    .collect(),
            )),
        }
    }
    pub fn by_id(&self, id: u64) -> Result<City, Error> {
        self.by_id
            .get(&id)
            .map(|&index| self.cities[index].clone())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::CityNotFound,
                    "That city ID is not in the local index. Search again.",
                )
            })
    }
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<City> {
        self.tree
            .nearest_neighbor(sphere(lat, lon))
            .map(|i| self.cities[i.index].clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualified_city_and_accents() {
        let c = Cities::load().unwrap();
        assert_eq!(c.resolve("Seattle, Washington").unwrap().state, "WA");
        assert_eq!(c.resolve("San José, CA").unwrap().ascii_name, "San Jose");
    }
    #[test]
    fn ambiguity_is_not_silently_guessed() {
        let c = Cities::load().unwrap();
        let e = c.resolve("Springfield").unwrap_err();
        assert_eq!(e.code, ErrorCode::AmbiguousCity);
        assert!(e.choices.unwrap().len() > 1);
    }
    #[test]
    fn ambiguity_lists_every_exact_match_by_population() {
        let c = Cities::load().unwrap();
        let franklin = c.resolve("Franklin").unwrap_err();
        assert_eq!(franklin.code, ErrorCode::AmbiguousCity);
        let choices = franklin.choices.unwrap();
        assert_eq!(choices.len(), 19);
        assert!(choices.iter().any(|c| c.state == "NC"));
        assert!(choices.iter().all(|c| c.name == "Franklin"));
        assert!(
            choices
                .windows(2)
                .all(|w| (w[0].population, std::cmp::Reverse(w[0].id))
                    >= (w[1].population, std::cmp::Reverse(w[1].id)))
        );
        let springfield = c.resolve("Springfield").unwrap_err().choices.unwrap();
        assert_eq!(springfield.len(), 20);
        // The index holds two Franklins in NC, so a state alone cannot choose between them.
        let nc = c.resolve("Franklin, NC").unwrap_err();
        assert_eq!(nc.code, ErrorCode::AmbiguousCity);
        assert_eq!(nc.choices.unwrap().len(), 2);
        assert_eq!(c.resolve("Franklin, TN").unwrap().state, "TN");
    }
    #[test]
    fn territories_use_their_two_letter_code_as_state() {
        let c = Cities::load().unwrap();
        let san_juan = c.resolve("San Juan, PR").unwrap();
        assert_eq!(
            (san_juan.state.as_str(), san_juan.country.as_str()),
            ("PR", "PR")
        );
        assert_eq!(san_juan.state_name, "Puerto Rico");
        assert_eq!(c.resolve("Hagåtña, Guam").unwrap().state, "GU");
        assert_eq!(c.resolve("San Juan, PR, US").unwrap().id, san_juan.id);
        assert_eq!(c.resolve("Hagåtña, GU, US").unwrap().state, "GU");
        for (code, name) in [
            ("AS", "American Samoa"),
            ("GU", "Guam"),
            ("MP", "Northern Mariana Islands"),
            ("PR", "Puerto Rico"),
            ("VI", "U.S. Virgin Islands"),
        ] {
            let in_territory: Vec<_> = c.cities.iter().filter(|c| c.country == code).collect();
            assert!(!in_territory.is_empty(), "{code}");
            assert!(
                in_territory
                    .iter()
                    .all(|c| c.state == code && c.state_name == name),
                "{code}"
            );
        }
        assert!(
            c.cities.iter().all(|c| c.state.len() == 2),
            "state is a two-letter code"
        );
    }
    #[test]
    fn not_found_suggestions_stay_limited() {
        let c = Cities::load().unwrap();
        let e = c.resolve("Spri").unwrap_err();
        assert_eq!(e.code, ErrorCode::CityNotFound);
        assert!(e.choices.unwrap().len() <= 10);
    }
    #[test]
    fn by_id_finds_every_city_and_rejects_unknown_ids() {
        let c = Cities::load().unwrap();
        for city in c.cities.iter() {
            assert_eq!(c.by_id(city.id).unwrap().id, city.id);
        }
        assert_eq!(c.by_id(0).unwrap_err().code, ErrorCode::CityNotFound);
        assert_eq!(c.by_id(u64::MAX).unwrap_err().code, ErrorCode::CityNotFound);
    }
    #[test]
    fn nearest_uses_sphere_not_flat_degrees() {
        let c = Cities::load().unwrap();
        assert_eq!(c.nearest(47.6062, -122.3321).unwrap().name, "Seattle");
    }
    #[test]
    fn suggestions_and_input_bounds() {
        let c = Cities::load().unwrap();
        assert!(!c.search("Seat").unwrap().is_empty());
        assert!(c.search("a").is_err());
        assert!(c.search("Seattle,,US").is_err());
        assert!(c.resolve("London, GB").is_err());
    }
}
