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

    let mut closed = coords.to_vec();
    if closed.first() != closed.last() {
        closed.push(coords[0]);
    }
    if closed.len() < 4 {
        return Err("polygon must have at least 3 points".into());
    }

    let mut area = 0.0;
    let mut cx = 0.0;
    let mut cy = 0.0;

    for i in 0..(closed.len() - 1) {
        let (x0, y0) = closed[i];
        let (x1, y1) = closed[i + 1];
        let cross = x0 * y1 - x1 * y0;
        area += cross;
        cx += (x0 + x1) * cross;
        cy += (y0 + y1) * cross;
    }

    area *= 0.5;
    if area.abs() < 1e-12 {
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for (x, y) in closed.iter().take(closed.len() - 1) {
            sum_x += x;
            sum_y += y;
        }
        let count = (closed.len() - 1) as f64;
        return Ok((sum_x / count, sum_y / count));
    }

    Ok((cx / (6.0 * area), cy / (6.0 * area)))
}

fn line_midpoint(coords: &[(f64, f64)]) -> Result<(f64, f64)> {
    if coords.len() < 2 {
        return Err("line must have at least 2 points".into());
    }

    let mut segments: Vec<((f64, f64), (f64, f64), f64)> = Vec::new();
    let mut total = 0.0;

    for i in 0..(coords.len() - 1) {
        let (x0, y0) = coords[i];
        let (x1, y1) = coords[i + 1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        segments.push(((x0, y0), (x1, y1), len));
        total += len;
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
    for (start, end, len) in segments {
        if acc + len >= halfway {
            let t = (halfway - acc) / len;
            return Ok((start.0 + (end.0 - start.0) * t, start.1 + (end.1 - start.1) * t));
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
    Some(PlaceNode {
        name,
        place_type,
        coord,
    })
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

fn nearest_place(point: (f64, f64), places: &[PlaceNode]) -> Option<PlaceNode> {
    const MAX_DISTANCE_KM: f64 = 75.0;
    let mut best: Option<(PlaceNode, f64)> = None;
    for place in places {
        let distance = haversine_km(point, place.coord);
        if distance > MAX_DISTANCE_KM {
            continue;
        }
        match best {
            None => best = Some((place.clone(), distance)),
            Some((_, best_distance)) if distance < best_distance => {
                best = Some((place.clone(), distance))
            }
            _ => {}
        }
    }
    best.map(|(place, _)| place)
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

fn is_in_country(tags: &Tags) -> Option<String> {
    if let Some(value) = tags.get("is_in:country") {
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    if let Some(value) = tags.get("is_in") {
        let parts: Vec<&str> = value.split(',').map(|part| part.trim()).collect();
        if let Some(last) = parts.last() {
            if !last.is_empty() {
                return Some(last.to_string());
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
}

#[derive(Default)]
struct NodeData {
    id: Option<i64>,
    coord: Option<(f64, f64)>,
    tags: Tags,
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

    for way in ways {
        if !way.tags.contains_key("highway") || !has_name_tags(&way.tags) {
            continue;
        }

        let names = collect_names(&way.tags);
        if names.is_empty() {
            continue;
        }

        let mut coords = Vec::new();
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

        let city_addr = way.tags.get("addr:city");
        let city_place = way.tags.get("addr:place");
        let city = city_addr.or(city_place);
        let city_boundary = None;
        let country_boundary = None;
        let place_match = nearest_place((center_lon, center_lat), &place_nodes);
        let city_place_node = place_match.as_ref().map(|place| place.name.clone());
        let city_place_type = place_match.as_ref().map(|place| place.place_type.clone());
        let city_is_in = is_in_city(&way.tags);
        let country_is_in = is_in_country(&way.tags);
        let city_resolved = resolve_first_non_empty(&[
            city.map(|value| value.as_str()),
            city_boundary.as_deref(),
            city_is_in.as_deref(),
            city_place_node.as_deref(),
        ]);
        let country_resolved =
            resolve_first_non_empty(&[country_boundary.as_deref(), country_is_in.as_deref()]);
        for name in names {
            writer.write_record([
                name,
                format!("{center_lon}"),
                format!("{center_lat}"),
                city.map(|value| value.as_str()).unwrap_or("").to_string(),
                city_addr
                    .map(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                city_place
                    .map(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                city_boundary.clone().unwrap_or_default(),
                country_boundary.clone().unwrap_or_default(),
                city_place_node.clone().unwrap_or_default(),
                city_place_type.clone().unwrap_or_default(),
                city_is_in.clone().unwrap_or_default(),
                country_is_in.clone().unwrap_or_default(),
                city_resolved.clone().unwrap_or_default(),
                country_resolved.clone().unwrap_or_default(),
                String::new(),
                String::new(),
            ])?;
        }
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
        OsmObj::Way(w) => w.tags.contains_key("highway") && has_name_tags(&w.tags),
        OsmObj::Node(n) => is_place_node(&n.tags),
        OsmObj::Relation(_) => false,
    })?;
    let place_nodes = collect_pbf_place_nodes(&objs);

    for obj in objs.values() {
        let way = match obj {
            OsmObj::Way(w) => w,
            _ => continue,
        };
        if !way.tags.contains_key("highway") || !has_name_tags(&way.tags) {
            continue;
        }

        let names = collect_names(&way.tags);
        if names.is_empty() {
            continue;
        }

        let mut coords = Vec::new();
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

        let city_addr = way.tags.get("addr:city");
        let city_place = way.tags.get("addr:place");
        let city = city_addr.or(city_place);
        let city_boundary = None;
        let country_boundary = None;
        let place_match = nearest_place((center_lon, center_lat), &place_nodes);
        let city_place_node = place_match.as_ref().map(|place| place.name.clone());
        let city_place_type = place_match.as_ref().map(|place| place.place_type.clone());
        let city_is_in = is_in_city(&way.tags);
        let country_is_in = is_in_country(&way.tags);
        let city_resolved = resolve_first_non_empty(&[
            city.map(|value| value.as_str()),
            city_boundary.as_deref(),
            city_is_in.as_deref(),
            city_place_node.as_deref(),
        ]);
        let country_resolved =
            resolve_first_non_empty(&[country_boundary.as_deref(), country_is_in.as_deref()]);
        for name in names {
            writer.write_record([
                name,
                format!("{center_lon:.7}"),
                format!("{center_lat:.7}"),
                city.map(|value| value.as_str()).unwrap_or("").to_string(),
                city_addr
                    .map(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                city_place
                    .map(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                city_boundary.clone().unwrap_or_default(),
                country_boundary.clone().unwrap_or_default(),
                city_place_node.clone().unwrap_or_default(),
                city_place_type.clone().unwrap_or_default(),
                city_is_in.clone().unwrap_or_default(),
                country_is_in.clone().unwrap_or_default(),
                city_resolved.clone().unwrap_or_default(),
                country_resolved.clone().unwrap_or_default(),
                String::new(),
                String::new(),
            ])?;
        }
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
        "city",
        "city_addr",
        "city_place",
        "city_boundary",
        "country_boundary",
        "city_place_node",
        "city_place_type",
        "city_is_in",
        "country_is_in",
        "city_resolved",
        "country_resolved",
        "admin_level_8",
        "admin_level_9",
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
                "city",
                "city_addr",
                "city_place",
                "city_boundary",
                "country_boundary",
                "city_place_node",
                "city_place_type",
                "city_is_in",
                "country_is_in",
                "city_resolved",
                "country_resolved",
                "admin_level_8",
                "admin_level_9"
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
        assert_eq!(open_row[6], "");
        assert_eq!(open_row[7], "");
        assert_eq!(open_row[14], "");

        let main_row = rows
            .iter()
            .skip(1)
            .find(|row| row[0] == "Main Street")
            .unwrap();
        assert_eq!(main_row[8], "Placetown");
        assert_eq!(main_row[9], "town");
        assert_eq!(main_row[10], "Placetown");
        assert_eq!(main_row[11], "Testland");
        assert_eq!(main_row[12], "Placetown");
        assert_eq!(main_row[13], "Testland");
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
}
