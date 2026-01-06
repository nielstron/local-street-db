use csv::Writer;
use osmpbfreader::{OsmId, OsmObj, OsmPbfReader, Tags};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::{HashMap, HashSet};
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

#[derive(Default)]
struct WayData {
    node_refs: Vec<i64>,
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
                        if let (Some(id), Some(lat), Some(lon)) = (id, lat, lon) {
                            nodes.insert(id, (lon, lat));
                        }
                    }
                    b"way" => {
                        current_way = Some(WayData::default());
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
                        ways.push(WayData::default());
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
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                if e.name().as_ref() == b"way" {
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
        _ => false,
    })?;

    for obj in objs.values() {
        let way = match obj {
            OsmObj::Way(w) => w,
            _ => continue,
        };

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
  <way id="10">
    <nd ref="1" />
    <nd ref="2" />
    <nd ref="3" />
    <nd ref="4" />
    <nd ref="1" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Main Street" />
    <tag k="alt_name" v="Old Main" />
  </way>
  <way id="11">
    <nd ref="4" />
    <nd ref="5" />
    <nd ref="6" />
    <tag k="highway" v="residential" />
    <tag k="name" v="Open Way" />
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
                "city_place"
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
