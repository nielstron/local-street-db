use csv::Writer;
use osmpbfreader::{OsmId, OsmObj, OsmPbfReader, Tags};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const NAME_KEYS: [&str; 6] = [
    "name",
    "alt_name",
    "old_name",
    "official_name",
    "loc_name",
    "short_name",
];

const NAME_PREFIXES: [&str; 6] = [
    "name:",
    "alt_name:",
    "old_name:",
    "official_name:",
    "loc_name:",
    "short_name:",
];

fn is_name_key(key: &str) -> bool {
    NAME_KEYS.contains(&key) || NAME_PREFIXES.iter().any(|prefix| key.starts_with(prefix))
}

fn split_names(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
}

fn add_names(value: &str, names: &mut Vec<String>, seen: &mut HashSet<String>) {
    for name in split_names(value) {
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
}

fn collect_names(tags: &Tags) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();

    for key in NAME_KEYS {
        if let Some(value) = tags.get(key) {
            if !value.is_empty() {
                add_names(value, &mut names, &mut seen);
            }
        }
    }

    for (key, value) in tags.iter() {
        if value.is_empty() {
            continue;
        }
        if NAME_KEYS.contains(&key.as_str()) {
            continue;
        }
        if NAME_PREFIXES.iter().any(|prefix| key.starts_with(prefix)) {
            add_names(value, &mut names, &mut seen);
        }
    }

    names
}

fn polygon_centroid(coords: &[(f64, f64)]) -> Result<(f64, f64)> {
    if coords.len() < 3 {
        return Err("polygon must have at least 3 points".into());
    }

    let is_closed = coords.len() >= 4 && coords.first() == coords.last();
    let mut area = 0.0;
    let mut cx = 0.0;
    let mut cy = 0.0;

    for i in 0..(coords.len() - 1) {
        let (x0, y0) = coords[i];
        let (x1, y1) = coords[i + 1];
        let cross = x0 * y1 - x1 * y0;
        area += cross;
        cx += (x0 + x1) * cross;
        cy += (y0 + y1) * cross;
    }
    if !is_closed {
        let (x0, y0) = coords[coords.len() - 1];
        let (x1, y1) = coords[0];
        let cross = x0 * y1 - x1 * y0;
        area += cross;
        cx += (x0 + x1) * cross;
        cy += (y0 + y1) * cross;
    }

    area *= 0.5;
    if area.abs() < 1e-12 {
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let count = if is_closed { coords.len() - 1 } else { coords.len() };
        for (x, y) in coords.iter().take(count) {
            sum_x += x;
            sum_y += y;
        }
        let count = count as f64;
        return Ok((sum_x / count, sum_y / count));
    }

    Ok((cx / (6.0 * area), cy / (6.0 * area)))
}

fn line_midpoint(coords: &[(f64, f64)]) -> Result<(f64, f64)> {
    if coords.len() < 2 {
        return Err("line must have at least 2 points".into());
    }

    let mut total = 0.0;
    for i in 0..(coords.len() - 1) {
        let (x0, y0) = coords[i];
        let (x1, y1) = coords[i + 1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        total += (dx * dx + dy * dy).sqrt();
    }

    if total == 0.0 {
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for (x, y) in coords {
            sum_x += x;
            sum_y += y;
        }
        let count = coords.len() as f64;
        return Ok((sum_x / count, sum_y / count));
    }

    let halfway = total / 2.0;
    let mut acc = 0.0;
    for i in 0..(coords.len() - 1) {
        let (x0, y0) = coords[i];
        let (x1, y1) = coords[i + 1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        if acc + len >= halfway {
            let t = (halfway - acc) / len;
            return Ok((x0 + (x1 - x0) * t, y0 + (y1 - y0) * t));
        }
        acc += len;
    }

    Ok(*coords.last().unwrap())
}

fn has_name_tags(tags: &Tags) -> bool {
    tags.iter()
        .any(|(key, value)| is_name_key(key) && !value.is_empty())
}

fn is_place_node(tags: &Tags) -> bool {
    let place = tags.get("place").map(|value| value.as_str()).unwrap_or("");
    let has_name = tags
        .get("name")
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    has_name
        && matches!(
            place,
            "city" | "town" | "village" | "hamlet" | "suburb" | "locality"
        )
}

fn place_node_from_tags(tags: &Tags, coord: (f64, f64)) -> Option<PlaceNode> {
    if !is_place_node(tags) {
        return None;
    }
    let name = tags.get("name")?.to_string();
    let place_type = tags.get("place")?.to_string();
    Some(PlaceNode::new(name, place_type, coord))
}

fn is_city_or_town(place_type: &str) -> bool {
    matches!(place_type, "city" | "town")
}

fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lon1, lat1) = a;
    let (lon2, lat2) = b;
    let r = 6371.0_f64;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();
    let sin_dlat = (dlat / 2.0).sin();
    let sin_dlon = (dlon / 2.0).sin();
    let h = sin_dlat * sin_dlat + lat1.cos() * lat2.cos() * sin_dlon * sin_dlon;
    2.0 * r * h.sqrt().asin()
}

fn path_length_km(coords: &[(f64, f64)]) -> f64 {
    if coords.len() < 2 {
        return 0.0;
    }
    let mut total = 0.0;
    for i in 0..(coords.len() - 1) {
        total += haversine_km(coords[i], coords[i + 1]);
    }
    total
}

const MAX_PLACE_DISTANCE_KM: f64 = 75.0;
const EARTH_RADIUS_KM: f64 = 6371.0;

#[derive(Copy, Clone)]
enum PlaceFilter {
    Any,
    CityTown,
}

struct PlaceIndex {
    places: Vec<PlaceNode>,
    grid: HashMap<(i32, i32), Vec<usize>>,
    cell_size_deg: f64,
}

impl PlaceIndex {
    fn new(places: Vec<PlaceNode>, cell_size_deg: f64) -> Self {
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (idx, place) in places.iter().enumerate() {
            let cell = Self::cell_for(place.coord, cell_size_deg);
            grid.entry(cell).or_default().push(idx);
        }
        Self {
            places,
            grid,
            cell_size_deg,
        }
    }

    fn nearest(&self, point: (f64, f64), filter: PlaceFilter) -> Option<&PlaceNode> {
        let (lon, lat) = point;
        let lat_rad = lat.to_radians();
        let lon_rad = lon.to_radians();
        let cos_lat = lat_rad.cos().abs();
        let delta_lat = MAX_PLACE_DISTANCE_KM / 111.0;
        let delta_lon = if cos_lat < 1e-6 {
            180.0
        } else {
            MAX_PLACE_DISTANCE_KM / (111.0 * cos_lat)
        };

        let min_cell = Self::cell_for((lon - delta_lon, lat - delta_lat), self.cell_size_deg);
        let max_cell = Self::cell_for((lon + delta_lon, lat + delta_lat), self.cell_size_deg);
        let mut best: Option<(&PlaceNode, f64)> = None;
        for x in min_cell.0..=max_cell.0 {
            for y in min_cell.1..=max_cell.1 {
                let Some(bucket) = self.grid.get(&(x, y)) else {
                    continue;
                };
                for &idx in bucket {
                    let place = &self.places[idx];
                    if matches!(filter, PlaceFilter::CityTown) && !place.is_city_town {
                        continue;
                    }
                    let distance = equirectangular_km(
                        lon_rad,
                        lat_rad,
                        cos_lat,
                        place.lon_rad,
                        place.lat_rad,
                        place.cos_lat,
                    );
                    if distance > MAX_PLACE_DISTANCE_KM {
                        continue;
                    }
                    match best {
                        None => best = Some((place, distance)),
                        Some((_, best_distance)) if distance < best_distance => {
                            best = Some((place, distance))
                        }
                        _ => {}
                    }
                }
            }
        }
        best.map(|(place, _)| place)
    }

    fn cell_for(coord: (f64, f64), cell_size_deg: f64) -> (i32, i32) {
        let x = (coord.0 / cell_size_deg).floor() as i32;
        let y = (coord.1 / cell_size_deg).floor() as i32;
        (x, y)
    }
}

fn equirectangular_km(
    lon1: f64,
    lat1: f64,
    cos_lat1: f64,
    lon2: f64,
    lat2: f64,
    cos_lat2: f64,
) -> f64 {
    let avg_cos = (cos_lat1 + cos_lat2) * 0.5;
    let x = (lon2 - lon1) * avg_cos;
    let y = lat2 - lat1;
    EARTH_RADIUS_KM * (x * x + y * y).sqrt()
}

fn is_in_city(tags: &Tags) -> Option<String> {
    for key in ["is_in:city", "is_in:town", "is_in:municipality", "is_in:locality"] {
        if let Some(value) = tags.get(key) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    if let Some(value) = tags.get("is_in") {
        let parts: Vec<&str> = value.split(',').map(|part| part.trim()).collect();
        if let Some(first) = parts.first() {
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
    }
    None
}

fn resolve_first_non_empty(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .find_map(|value| value.and_then(|text| if text.is_empty() { None } else { Some(text) }))
        .map(|value| value.to_string())
}

fn has_tag_value(tags: &Tags, key: &str, values: &[&str]) -> bool {
    tags.get(key)
        .map(|value| values.contains(&value.as_str()))
        .unwrap_or(false)
}

fn has_tag(tags: &Tags, key: &str) -> bool {
    tags.get(key).map(|value| !value.is_empty()).unwrap_or(false)
}

fn is_airport(tags: &Tags) -> bool {
    has_tag_value(tags, "aeroway", &["aerodrome", "airport", "terminal"])
}

fn is_train_station(tags: &Tags) -> bool {
    has_tag_value(tags, "railway", &["station", "halt"])
        || has_tag_value(tags, "public_transport", &["station"])
}

fn is_bus_stop(tags: &Tags) -> bool {
    has_tag_value(tags, "highway", &["bus_stop"])
        || has_tag_value(tags, "public_transport", &["platform", "stop_position"])
}

fn is_major_sight(tags: &Tags) -> bool {
    if !has_name_tags(tags) {
        return false;
    }
    let has_wiki = tags.contains_key("wikipedia") || tags.contains_key("wikidata");
    if !has_wiki {
        return false;
    }
    let has_tourism = has_tag(tags, "tourism");
    let has_historic = has_tag(tags, "historic");
    let has_man_made = has_tag(tags, "man_made");
    let has_landmark = has_tag(tags, "landmark");
    let has_tower = has_tag(tags, "tower:type");
    has_tourism || has_historic || has_man_made || has_landmark || has_tower
}

fn is_poi(tags: &Tags) -> bool {
    if !has_name_tags(tags) {
        return false;
    }
    is_airport(tags) || is_train_station(tags) || is_bus_stop(tags) || is_major_sight(tags)
}

fn resolve_city_fields(
    tags: &Tags,
    center: (f64, f64),
    place_index: &PlaceIndex,
) -> (String, String, String, String) {
    let city_addr = tags.get("addr:city");
    let city_place = tags.get("addr:place");
    let city = city_addr.or(city_place);
    let city_boundary: Option<String> = None;
    let place_match = place_index.nearest(center, PlaceFilter::Any);
    let city_place_node = place_match.as_ref().map(|place| place.name.clone());
    let city_place_type = place_match.as_ref().map(|place| place.place_type.clone());
    let city_place_city = match place_match.as_ref() {
        Some(place) if is_city_or_town(&place.place_type) => Some(place.name.clone()),
        Some(_) => place_index
            .nearest(center, PlaceFilter::CityTown)
            .map(|place| place.name.clone()),
        None => None,
    };
    let city_is_in = is_in_city(tags);
    let city_resolved = resolve_first_non_empty(&[
        city.map(|value| value.as_str()),
        city_boundary.as_deref(),
        city_is_in.as_deref(),
        city_place_city.as_deref(),
        city_place_node.as_deref(),
    ]);
    (
        city_place_node.unwrap_or_default(),
        city_place_type.unwrap_or_default(),
        city_place_city.unwrap_or_default(),
        city_resolved.unwrap_or_default(),
    )
}


fn collect_pbf_place_nodes(objs: &BTreeMap<OsmId, OsmObj>) -> Vec<PlaceNode> {
    let mut places = Vec::new();
    for obj in objs.values() {
        if let OsmObj::Node(node) = obj {
            if let Some(place) = place_node_from_tags(&node.tags, (node.lon(), node.lat())) {
                places.push(place);
            }
        }
    }
    places
}

#[derive(Default, Clone)]
struct WayData {
    id: Option<i64>,
    node_refs: Vec<i64>,
    tags: Tags,
}

#[derive(Clone)]
struct PlaceNode {
    name: String,
    place_type: String,
    coord: (f64, f64),
    lat_rad: f64,
    lon_rad: f64,
    cos_lat: f64,
    is_city_town: bool,
}

impl PlaceNode {
    fn new(name: String, place_type: String, coord: (f64, f64)) -> Self {
        let lat_rad = coord.1.to_radians();
        let lon_rad = coord.0.to_radians();
        let cos_lat = lat_rad.cos();
        let is_city_town = is_city_or_town(&place_type);
        Self {
            name,
            place_type,
            coord,
            lat_rad,
            lon_rad,
            cos_lat,
            is_city_town,
        }
    }
}

#[derive(Default)]
struct NodeData {
    id: Option<i64>,
    coord: Option<(f64, f64)>,
    tags: Tags,
}

#[derive(Clone)]
struct StreetEntry {
    name: String,
    center_lon: f64,
    center_lat: f64,
    length_km: f64,
    city_place_node: String,
    city_place_type: String,
    city_place_city: String,
    city_resolved: String,
}

const MERGE_DISTANCE_KM: f64 = 0.2;

fn merge_city_key(entry: &StreetEntry) -> String {
    if !entry.city_resolved.is_empty() {
        return entry.city_resolved.clone();
    }
    if !entry.city_place_city.is_empty() {
        return entry.city_place_city.clone();
    }
    if !entry.city_place_node.is_empty() {
        return entry.city_place_node.clone();
    }
    String::new()
}

fn pick_mode(entries: &[StreetEntry], indices: &[usize], getter: fn(&StreetEntry) -> &str) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for idx in indices {
        let value = getter(&entries[*idx]);
        if value.is_empty() {
            continue;
        }
        *counts.entry(value.to_string()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value)
        .unwrap_or_default()
}

fn merge_cluster(entries: &[StreetEntry], indices: &[usize]) -> StreetEntry {
    let mut weighted_lon = 0.0;
    let mut weighted_lat = 0.0;
    let mut weight_sum = 0.0;
    let mut length_sum = 0.0;

    for idx in indices {
        let entry = &entries[*idx];
        let weight = if entry.length_km > 0.0 { entry.length_km } else { 1.0 };
        weighted_lon += entry.center_lon * weight;
        weighted_lat += entry.center_lat * weight;
        weight_sum += weight;
        length_sum += entry.length_km;
    }

    let center_lon = if weight_sum > 0.0 {
        weighted_lon / weight_sum
    } else {
        entries[indices[0]].center_lon
    };
    let center_lat = if weight_sum > 0.0 {
        weighted_lat / weight_sum
    } else {
        entries[indices[0]].center_lat
    };

    let name = entries[indices[0]].name.clone();
    let city_place_node = pick_mode(entries, indices, |e| e.city_place_node.as_str());
    let city_place_type = pick_mode(entries, indices, |e| e.city_place_type.as_str());
    let city_place_city = pick_mode(entries, indices, |e| e.city_place_city.as_str());
    let city_resolved = pick_mode(entries, indices, |e| e.city_resolved.as_str());

    StreetEntry {
        name,
        center_lon,
        center_lat,
        length_km: length_sum,
        city_place_node,
        city_place_type,
        city_place_city,
        city_resolved,
    }
}

fn merge_entries(entries: Vec<StreetEntry>) -> Vec<StreetEntry> {
    let mut grouped: Vec<((String, String), Vec<StreetEntry>)> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for entry in entries {
        let key = (entry.name.clone(), merge_city_key(&entry));
        if let Some(&position) = index.get(&key) {
            grouped[position].1.push(entry);
        } else {
            index.insert(key.clone(), grouped.len());
            grouped.push((key, vec![entry]));
        }
    }

    let mut merged = Vec::new();
    for (_, group) in grouped {
        let mut remaining = vec![true; group.len()];
        for i in 0..group.len() {
            if !remaining[i] {
                continue;
            }
            remaining[i] = false;
            let mut cluster = vec![i];
            let mut queue = vec![i];

            while let Some(idx) = queue.pop() {
                let base = (group[idx].center_lon, group[idx].center_lat);
                for j in 0..group.len() {
                    if !remaining[j] {
                        continue;
                    }
                    let other = (group[j].center_lon, group[j].center_lat);
                    if haversine_km(base, other) <= MERGE_DISTANCE_KM {
                        remaining[j] = false;
                        queue.push(j);
                        cluster.push(j);
                    }
                }
            }

            merged.push(merge_cluster(&group, &cluster));
        }
    }

    merged
}

fn get_attr_value(event: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attr in event.attributes().with_checks(false) {
        let attr = attr?;
        if attr.key.as_ref() == key {
            return Ok(Some(attr.unescape_value()?.to_string()));
        }
    }
    Ok(None)
}

fn extract_osm_xml_to_writer(input_path: &Path, writer: &mut Writer<File>) -> Result<()> {
    let file = File::open(input_path)?;
    let mut reader = Reader::from_reader(BufReader::new(file));
    reader.trim_text(true);

    let mut nodes: HashMap<i64, (f64, f64)> = HashMap::new();
    let mut ways: Vec<WayData> = Vec::new();
    let mut place_nodes: Vec<PlaceNode> = Vec::new();
    let mut poi_nodes: Vec<NodeData> = Vec::new();
    let mut current_node: Option<NodeData> = None;
    let mut current_way: Option<WayData> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"node" => {
                        let id = get_attr_value(&e, b"id")?
                            .and_then(|value| value.parse::<i64>().ok());
                        let lat = get_attr_value(&e, b"lat")?
                            .and_then(|value| value.parse::<f64>().ok());
                        let lon = get_attr_value(&e, b"lon")?
                            .and_then(|value| value.parse::<f64>().ok());
                        if id.is_some() && lat.is_some() && lon.is_some() {
                            current_node = Some(NodeData {
                                id,
                                coord: Some((lon.unwrap(), lat.unwrap())),
                                tags: Tags::new(),
                            });
                        }
                    }
                    b"way" => {
                        let id = get_attr_value(&e, b"id")?
                            .and_then(|value| value.parse::<i64>().ok());
                        current_way = Some(WayData {
                            id,
                            ..WayData::default()
                        });
                    }
                    b"nd" => {
                        if let Some(way) = current_way.as_mut() {
                            if let Some(reference) = get_attr_value(&e, b"ref")?
                                .and_then(|value| value.parse::<i64>().ok())
                            {
                                way.node_refs.push(reference);
                            }
                        }
                    }
                    b"tag" => {
                        if let Some(way) = current_way.as_mut() {
                            let key = get_attr_value(&e, b"k")?;
                            let value = get_attr_value(&e, b"v")?;
                            if let (Some(key), Some(value)) = (key, value) {
                                way.tags.insert(key.into(), value.into());
                            }
                        }
                        if let Some(node) = current_node.as_mut() {
                            let key = get_attr_value(&e, b"k")?;
                            let value = get_attr_value(&e, b"v")?;
                            if let (Some(key), Some(value)) = (key, value) {
                                node.tags.insert(key.into(), value.into());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"node" => {
                        let id = get_attr_value(&e, b"id")?
                            .and_then(|value| value.parse::<i64>().ok());
                        let lat = get_attr_value(&e, b"lat")?
                            .and_then(|value| value.parse::<f64>().ok());
                        let lon = get_attr_value(&e, b"lon")?
                            .and_then(|value| value.parse::<f64>().ok());
                        if let (Some(id), Some(lat), Some(lon)) = (id, lat, lon) {
                            nodes.insert(id, (lon, lat));
                        }
                    }
                    b"way" => {
                        let id = get_attr_value(&e, b"id")?
                            .and_then(|value| value.parse::<i64>().ok());
                        let way = WayData {
                            id,
                            ..WayData::default()
                        };
                        ways.push(way);
                    }
                    b"nd" => {
                        if let Some(way) = current_way.as_mut() {
                            if let Some(reference) = get_attr_value(&e, b"ref")?
                                .and_then(|value| value.parse::<i64>().ok())
                            {
                                way.node_refs.push(reference);
                            }
                        }
                    }
                    b"tag" => {
                        if let Some(way) = current_way.as_mut() {
                            let key = get_attr_value(&e, b"k")?;
                            let value = get_attr_value(&e, b"v")?;
                            if let (Some(key), Some(value)) = (key, value) {
                                way.tags.insert(key.into(), value.into());
                            }
                        }
                        if let Some(node) = current_node.as_mut() {
                            let key = get_attr_value(&e, b"k")?;
                            let value = get_attr_value(&e, b"v")?;
                            if let (Some(key), Some(value)) = (key, value) {
                                node.tags.insert(key.into(), value.into());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                if e.name().as_ref() == b"node" {
                    if let Some(node) = current_node.take() {
                        if let (Some(id), Some(coord)) = (node.id, node.coord) {
                            nodes.insert(id, coord);
                            if let Some(place_node) = place_node_from_tags(&node.tags, coord) {
                                place_nodes.push(place_node);
                            }
                            if is_poi(&node.tags) {
                                poi_nodes.push(node);
                            }
                        }
                    }
                } else if e.name().as_ref() == b"way" {
                    if let Some(way) = current_way.take() {
                        ways.push(way);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    let place_index = PlaceIndex::new(place_nodes, 1.0);
    let mut entries: Vec<StreetEntry> = Vec::new();
    for node in poi_nodes {
        let coord = match node.coord {
            Some(coord) => coord,
            None => continue,
        };
        let names = collect_names(&node.tags);
        if names.is_empty() {
            continue;
        }
        let (city_place_node, city_place_type, city_place_city, city_resolved) =
            resolve_city_fields(&node.tags, coord, &place_index);
        for name in names {
            entries.push(StreetEntry {
                name,
                center_lon: coord.0,
                center_lat: coord.1,
                length_km: 0.0,
                city_place_node: city_place_node.clone(),
                city_place_type: city_place_type.clone(),
                city_place_city: city_place_city.clone(),
                city_resolved: city_resolved.clone(),
            });
        }
    }
    for way in ways {
        let is_street = way.tags.contains_key("highway") && has_name_tags(&way.tags);
        let is_poi_way = is_poi(&way.tags);
        if !is_street && !is_poi_way {
            continue;
        }

        let names = collect_names(&way.tags);
        if names.is_empty() {
            continue;
        }

        let mut coords = Vec::with_capacity(way.node_refs.len());
        let mut valid = true;
        for node_id in &way.node_refs {
            if let Some(coord) = nodes.get(node_id) {
                coords.push(*coord);
            } else {
                valid = false;
                break;
            }
        }
        if !valid {
            continue;
        }

        let is_closed = way.node_refs.len() >= 2 && way.node_refs.first() == way.node_refs.last();
        let (center_lon, center_lat) = if is_closed {
            if coords.len() < 4 {
                continue;
            }
            match polygon_centroid(&coords) {
                Ok(value) => value,
                Err(_) => continue,
            }
        } else {
            if coords.len() < 2 {
                continue;
            }
            match line_midpoint(&coords) {
                Ok(value) => value,
                Err(_) => continue,
            }
        };

        let (city_place_node, city_place_type, city_place_city, city_resolved) =
            resolve_city_fields(&way.tags, (center_lon, center_lat), &place_index);
        let length_km = if is_street { path_length_km(&coords) } else { 0.0 };
        for name in names {
            entries.push(StreetEntry {
                name,
                center_lon,
                center_lat,
                length_km,
                city_place_node: city_place_node.clone(),
                city_place_type: city_place_type.clone(),
                city_place_city: city_place_city.clone(),
                city_resolved: city_resolved.clone(),
            });
        }
    }

    for entry in merge_entries(entries) {
        writer.write_record([
            entry.name,
            format!("{}", entry.center_lon),
            format!("{}", entry.center_lat),
            entry.city_place_node,
            entry.city_place_type,
            entry.city_place_city,
            entry.city_resolved,
        ])?;
    }

    Ok(())
}

fn find_default_pbf(folder: &Path) -> Result<PathBuf> {
    let mut pbfs = Vec::new();
    for entry in folder.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("pbf") {
            pbfs.push(path);
        }
    }
    pbfs.sort();

    match pbfs.len() {
        0 => Err("no .pbf files found in current directory".into()),
        1 => Ok(pbfs.remove(0)),
        _ => Err("multiple .pbf files found; pass --input explicitly".into()),
    }
}

fn extract_pbf_to_writer(input_path: &Path, writer: &mut Writer<File>) -> Result<()> {
    let file = File::open(input_path)?;
    let mut pbf = OsmPbfReader::new(file);

    let objs = pbf.get_objs_and_deps(|obj| match obj {
        OsmObj::Way(w) => {
            (w.tags.contains_key("highway") && has_name_tags(&w.tags)) || is_poi(&w.tags)
        }
        OsmObj::Node(n) => is_place_node(&n.tags) || is_poi(&n.tags),
        OsmObj::Relation(_) => false,
    })?;
    let place_nodes = collect_pbf_place_nodes(&objs);
    let place_index = PlaceIndex::new(place_nodes, 1.0);

    let mut entries: Vec<StreetEntry> = Vec::new();
    for obj in objs.values() {
        match obj {
            OsmObj::Way(way) => {
                let is_street = way.tags.contains_key("highway") && has_name_tags(&way.tags);
                let is_poi_way = is_poi(&way.tags);
                if !is_street && !is_poi_way {
                    continue;
                }

                let names = collect_names(&way.tags);
                if names.is_empty() {
                    continue;
                }

                let mut coords = Vec::with_capacity(way.nodes.len());
                let mut valid = true;
                for node_id in &way.nodes {
                    match objs.get(&OsmId::Node(node_id.clone())) {
                        Some(OsmObj::Node(node)) => {
                            coords.push((node.lon(), node.lat()));
                        }
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if !valid {
                    continue;
                }

                let is_closed = way.nodes.len() >= 2 && way.nodes.first() == way.nodes.last();
                let (center_lon, center_lat) = if is_closed {
                    if coords.len() < 4 {
                        continue;
                    }
                    match polygon_centroid(&coords) {
                        Ok(value) => value,
                        Err(_) => continue,
                    }
                } else {
                    if coords.len() < 2 {
                        continue;
                    }
                    match line_midpoint(&coords) {
                        Ok(value) => value,
                        Err(_) => continue,
                    }
                };

                let (city_place_node, city_place_type, city_place_city, city_resolved) =
                    resolve_city_fields(&way.tags, (center_lon, center_lat), &place_index);
                let length_km = if is_street { path_length_km(&coords) } else { 0.0 };
                for name in names {
                    entries.push(StreetEntry {
                        name,
                        center_lon,
                        center_lat,
                        length_km,
                        city_place_node: city_place_node.clone(),
                        city_place_type: city_place_type.clone(),
                        city_place_city: city_place_city.clone(),
                        city_resolved: city_resolved.clone(),
                    });
                }
            }
            OsmObj::Node(node) => {
                if !is_poi(&node.tags) {
                    continue;
                }
                let names = collect_names(&node.tags);
                if names.is_empty() {
                    continue;
                }
                let center = (node.lon(), node.lat());
                let (city_place_node, city_place_type, city_place_city, city_resolved) =
                    resolve_city_fields(&node.tags, center, &place_index);
                for name in names {
                    entries.push(StreetEntry {
                        name,
                        center_lon: center.0,
                        center_lat: center.1,
                        length_km: 0.0,
                        city_place_node: city_place_node.clone(),
                        city_place_type: city_place_type.clone(),
                        city_place_city: city_place_city.clone(),
                        city_resolved: city_resolved.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    for entry in merge_entries(entries) {
        writer.write_record([
            entry.name,
            format!("{:.7}", entry.center_lon),
            format!("{:.7}", entry.center_lat),
            entry.city_place_node,
            entry.city_place_type,
            entry.city_place_city,
            entry.city_resolved,
        ])?;
    }

    Ok(())
}

fn extract_to_csv(input_path: &Path, output_path: &Path) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut writer = Writer::from_path(output_path)?;
    writer.write_record([
        "streetname",
        "center_lon",
        "center_lat",
        "city_place_node",
        "city_place_type",
        "city_place_city",
        "city_resolved",
    ])?;

    let ext = input_path.extension().and_then(|value| value.to_str());
    match ext {
        Some("osm") => extract_osm_xml_to_writer(input_path, &mut writer)?,
        _ => extract_pbf_to_writer(input_path, &mut writer)?,
    }

    writer.flush()?;
    Ok(())
}

fn parse_args() -> Result<(PathBuf, PathBuf)> {
    let mut input = None;
    let mut output = PathBuf::from("street_polygons.csv");

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input = Some(
                    args.next()
                        .ok_or("--input requires a path")
                        .map(PathBuf::from)?,
                );
            }
            "--output" => {
                output = args
                    .next()
                    .ok_or("--output requires a path")
                    .map(PathBuf::from)?;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: extract_street_polygons [--input FILE] [--output FILE]\n\n"
                );
                println!(
                    "--input   Path to a .pbf or .osm file. Defaults to the only .pbf in the current folder."
                );
                println!("--output  Output CSV path. Defaults to street_polygons.csv.");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    let input_path = match input {
        Some(path) => path,
        None => find_default_pbf(&env::current_dir()?)?,
    };

    Ok((input_path, output))
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let (input_path, output_path) = parse_args()?;
    extract_to_csv(&input_path, &output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use csv::ReaderBuilder;
    use tempfile::tempdir;

    const OSM_SAMPLE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<osm version="0.6" generator="test">
  <node id="1" lat="0.0" lon="0.0" />
  <node id="2" lat="0.0" lon="1.0" />
  <node id="3" lat="1.0" lon="1.0" />
  <node id="4" lat="1.0" lon="0.0" />
  <node id="5" lat="2.0" lon="0.0" />
  <node id="6" lat="2.0" lon="1.0" />
  <node id="7" lat="0.5" lon="0.5">
    <tag k="place" v="town" />
    <tag k="name" v="Placetown" />
  </node>
  <node id="100" lat="-1.0" lon="-1.0" />
  <node id="101" lat="-1.0" lon="3.0" />
  <node id="102" lat="3.0" lon="3.0" />
  <node id="103" lat="3.0" lon="-1.0" />
  <node id="200" lat="-5.0" lon="-5.0" />
  <node id="201" lat="-5.0" lon="5.0" />
  <node id="202" lat="5.0" lon="5.0" />
  <node id="203" lat="5.0" lon="-5.0" />
  <way id="10">
    <nd ref="1" />
    <nd ref="2" />
    <nd ref="3" />
    <nd ref="4" />
    <nd ref="1" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Main Street" />
    <tag k="alt_name" v="Old Main" />
    <tag k="is_in" v="Placetown, Testland" />
  </way>
  <way id="11">
    <nd ref="4" />
    <nd ref="5" />
    <nd ref="6" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Open Way" />
  </way>
  <way id="300">
    <nd ref="100" />
    <nd ref="101" />
    <nd ref="102" />
    <nd ref="103" />
    <nd ref="100" />
    <tag k="boundary" v="administrative" />
    <tag k="admin_level" v="8" />
    <tag k="name" v="Testville" />
  </way>
  <way id="301">
    <nd ref="200" />
    <nd ref="201" />
    <nd ref="202" />
    <nd ref="203" />
    <nd ref="200" />
    <tag k="boundary" v="administrative" />
    <tag k="admin_level" v="2" />
    <tag k="name" v="Testland" />
  </way>
</osm>
"#;

    const OSM_MULTI_NAME: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<osm version="0.6" generator="test">
  <node id="1" lat="0.0" lon="0.0" />
  <node id="2" lat="0.0" lon="2.0" />
  <node id="3" lat="2.0" lon="2.0" />
  <node id="4" lat="2.0" lon="0.0" />
  <way id="20">
    <nd ref="1" />
    <nd ref="2" />
    <nd ref="3" />
    <nd ref="4" />
    <nd ref="1" />
    <tag k="highway" v="primary" />
    <tag k="name" v="First;Second" />
    <tag k="name:de" v="Erste Straße" />
  </way>
</osm>
"#;

    const OSM_LARGER_PLACE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<osm version="0.6" generator="test">
  <node id="1" lat="0.0" lon="0.0" />
  <node id="2" lat="0.0" lon="0.2" />
  <node id="10" lat="0.0" lon="0.1">
    <tag k="place" v="hamlet" />
    <tag k="name" v="Tinyham" />
  </node>
  <node id="11" lat="0.0" lon="0.5">
    <tag k="place" v="town" />
    <tag k="name" v="Bigtown" />
  </node>
  <way id="20">
    <nd ref="1" />
    <nd ref="2" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Hamlet Road" />
  </way>
</osm>
"#;

    const OSM_MERGE_NEARBY: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<osm version="0.6" generator="test">
  <node id="1" lat="0.0" lon="0.0" />
  <node id="2" lat="0.001" lon="0.0" />
  <node id="3" lat="0.002" lon="0.0" />
  <way id="40">
    <nd ref="1" />
    <nd ref="2" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Dave Burns Drive" />
  </way>
  <way id="41">
    <nd ref="2" />
    <nd ref="3" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Dave Burns Drive" />
  </way>
</osm>
"#;

    const OSM_POI: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<osm version="0.6" generator="test">
  <node id="1" lat="48.8584" lon="2.2945">
    <tag k="name" v="Eiffel Tower" />
    <tag k="tourism" v="attraction" />
    <tag k="wikipedia" v="en:Eiffel_Tower" />
  </node>
  <node id="2" lat="48.0" lon="2.0">
    <tag k="name" v="Local Statue" />
    <tag k="tourism" v="attraction" />
  </node>
  <node id="3" lat="40.0" lon="-73.0">
    <tag k="name" v="Central Station" />
    <tag k="railway" v="station" />
  </node>
  <node id="4" lat="41.0" lon="-74.0">
    <tag k="name" v="Main Bus Stop" />
    <tag k="highway" v="bus_stop" />
  </node>
  <node id="5" lat="42.0" lon="-75.0">
    <tag k="name" v="City Airport" />
    <tag k="aeroway" v="aerodrome" />
  </node>
</osm>
"#;

    #[test]
    fn split_and_collect_names() {
        let mut tags = Tags::new();
        tags.insert("name".into(), "Main St;Second St".into());
        tags.insert("alt_name".into(), "Alt".into());
        tags.insert("foo".into(), "bar".into());

        let names = collect_names(&tags);
        assert_eq!(names, vec!["Main St", "Second St", "Alt"]);
    }

    #[test]
    fn polygon_centroid_square() {
        let coords = vec![(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0), (0.0, 0.0)];
        let (cx, cy) = polygon_centroid(&coords).unwrap();
        assert!((cx - 1.0).abs() < 1e-9);
        assert!((cy - 1.0).abs() < 1e-9);
    }

    #[test]
    fn line_midpoint_basic() {
        let coords = vec![(0.0, 0.0), (4.0, 0.0)];
        let (mx, my) = line_midpoint(&coords).unwrap();
        assert!((mx - 2.0).abs() < 1e-9);
        assert!(my.abs() < 1e-9);
    }

    #[test]
    fn place_index_picks_nearest_and_filters() {
        let places = vec![
            PlaceNode::new("Near".to_string(), "town".to_string(), (0.0, 0.0)),
            PlaceNode::new("Far".to_string(), "hamlet".to_string(), (5.0, 5.0)),
        ];
        let index = PlaceIndex::new(places, 1.0);

        let nearest = index
            .nearest((0.1, 0.1), PlaceFilter::Any)
            .unwrap();
        assert_eq!(nearest.name, "Near");

        let filtered = index
            .nearest((0.1, 0.1), PlaceFilter::CityTown)
            .map(|place| place.name.clone());
        assert_eq!(filtered.as_deref(), Some("Near"));
    }

    #[test]
    fn extract_to_csv_from_osm() {
        let dir = tempdir().unwrap();
        let osm_path = dir.path().join("sample.osm");
        let out_path = dir.path().join("out.csv");
        std::fs::write(&osm_path, OSM_SAMPLE).unwrap();

        extract_to_csv(&osm_path, &out_path).unwrap();

        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .from_path(&out_path)
            .unwrap();
        let rows: Vec<Vec<String>> = reader
            .records()
            .map(|row| row.unwrap().iter().map(|value| value.to_string()).collect())
            .collect();

        assert_eq!(
            rows[0],
            vec![
                "streetname",
                "center_lon",
                "center_lat",
                "city_place_node",
                "city_place_type",
                "city_place_city",
                "city_resolved",
            ]
        );
        let names: Vec<&str> = rows[1..].iter().map(|row| row[0].as_str()).collect();
        assert_eq!(names, vec!["Main Street", "Old Main", "Open Way"]);

        let open_row = rows
            .iter()
            .skip(1)
            .find(|row| row[0] == "Open Way")
            .unwrap();
        assert_eq!(open_row[1], "0");
        assert_eq!(open_row[2], "2");
        assert_eq!(open_row[3], "");
        assert_eq!(open_row[4], "");
        assert_eq!(open_row[5], "");
        assert_eq!(open_row[6], "");

        let main_row = rows
            .iter()
            .skip(1)
            .find(|row| row[0] == "Main Street")
            .unwrap();
        assert_eq!(main_row[3], "Placetown");
        assert_eq!(main_row[4], "town");
        assert_eq!(main_row[5], "Placetown");
        assert_eq!(main_row[6], "Placetown");
    }

    #[test]
    fn extract_to_csv_splits_multi_names() {
        let dir = tempdir().unwrap();
        let osm_path = dir.path().join("multi.osm");
        let out_path = dir.path().join("out.csv");
        std::fs::write(&osm_path, OSM_MULTI_NAME).unwrap();

        extract_to_csv(&osm_path, &out_path).unwrap();

        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .from_path(&out_path)
            .unwrap();
        let rows: Vec<Vec<String>> = reader
            .records()
            .map(|row| row.unwrap().iter().map(|value| value.to_string()).collect())
            .collect();

        let names: Vec<&str> = rows[1..].iter().map(|row| row[0].as_str()).collect();
        assert_eq!(names, vec!["First", "Second", "Erste Straße"]);
    }

    #[test]
    fn extract_to_csv_promotes_town_or_city() {
        let dir = tempdir().unwrap();
        let osm_path = dir.path().join("larger_place.osm");
        let out_path = dir.path().join("out.csv");
        std::fs::write(&osm_path, OSM_LARGER_PLACE).unwrap();

        extract_to_csv(&osm_path, &out_path).unwrap();

        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .from_path(&out_path)
            .unwrap();
        let rows: Vec<Vec<String>> = reader
            .records()
            .map(|row| row.unwrap().iter().map(|value| value.to_string()).collect())
            .collect();

        let hamlet_row = rows
            .iter()
            .skip(1)
            .find(|row| row[0] == "Hamlet Road")
            .unwrap();
        assert_eq!(hamlet_row[3], "Tinyham");
        assert_eq!(hamlet_row[4], "hamlet");
        assert_eq!(hamlet_row[5], "Bigtown");
        assert_eq!(hamlet_row[6], "Bigtown");
    }

    #[test]
    fn extract_to_csv_merges_nearby_segments() {
        let dir = tempdir().unwrap();
        let osm_path = dir.path().join("merge.osm");
        let out_path = dir.path().join("out.csv");
        std::fs::write(&osm_path, OSM_MERGE_NEARBY).unwrap();

        extract_to_csv(&osm_path, &out_path).unwrap();

        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .from_path(&out_path)
            .unwrap();
        let rows: Vec<Vec<String>> = reader
            .records()
            .map(|row| row.unwrap().iter().map(|value| value.to_string()).collect())
            .collect();

        let data_rows: Vec<&Vec<String>> = rows.iter().skip(1).collect();
        assert_eq!(data_rows.len(), 1);
        assert_eq!(data_rows[0][0], "Dave Burns Drive");

        let lat: f64 = data_rows[0][2].parse().unwrap();
        assert!((lat - 0.001).abs() < 1e-9);
    }

    #[test]
    fn extract_to_csv_includes_poi_and_filters_minor_sights() {
        let dir = tempdir().unwrap();
        let osm_path = dir.path().join("poi.osm");
        let out_path = dir.path().join("out.csv");
        std::fs::write(&osm_path, OSM_POI).unwrap();

        extract_to_csv(&osm_path, &out_path).unwrap();

        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .from_path(&out_path)
            .unwrap();
        let mut names: Vec<String> = reader
            .records()
            .skip(1)
            .map(|row| row.unwrap()[0].to_string())
            .collect();
        names.sort();
        let expected: Vec<String> = vec![
            "Central Station",
            "City Airport",
            "Eiffel Tower",
            "Main Bus Stop",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(names, expected);
    }
}
