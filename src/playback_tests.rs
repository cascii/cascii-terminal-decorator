use super::*;
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn manifest(widths: &[u32], frames: usize, map: bool) -> Value {
    let variants = widths.iter().map(|&columns| json!({"columns": columns, "rows": vec![columns; frames]})).collect::<Vec<_>>();
    let mut value = json!({
        "format": "cascii-multisize", "schema_version": 1, "frame_count": frames, "fps": 24,
        "default_columns": widths[0], "font_ratio": 0.7, "luminance": 20, "ascii_chars": " xy",
        "text": true, "color": false, "outputs": ["frames.bin"], "variants": variants
    });
    if map {
        value["schema_version"] = json!(2);
        value["column_map"] = json!((0..frames).map(|i| widths[i % widths.len()]).collect::<Vec<_>>());
    }
    value
}

fn binary(widths: &[u32], count: usize, map: bool) -> Vec<u8> {
    let metadata = serde_json::to_vec(&manifest(widths, count, map)).unwrap();
    let mut bytes = b"CSMV".to_vec();
    let mut payload = Vec::new();
    for word in [if map {2} else {1}, metadata.len() as u32, (count * widths.len()) as u32] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&metadata);
    for index in 0..count {
        for &width in widths {
            let line = if index % 2 == 0 {"x"} else {"y"}.repeat(width as usize) + "\n";
            let frame = line.repeat(width as usize);
            bytes.extend_from_slice(&width.to_le_bytes());
            bytes.extend_from_slice(&(index as u32).to_le_bytes());
            bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(frame.as_bytes());
        }
    }
    bytes.extend_from_slice(&payload);
    bytes
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn responsive_selection_keeps_time_and_uses_drawable_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frames.bin");
    fs::write(&path, binary(&[2, 4, 6], 3, true)).unwrap();
    let mut source = Frames::open(&Input::Path(path)).unwrap();
    let m = source.manifest().unwrap();
    assert_eq!(m.fps, 24);
    assert_eq!(Detail::Auto.columns(m, 1, 20, 3), 2);
    assert_eq!(Detail::Auto.columns(m, 1, 20, 5), 4);
    assert_eq!(Detail::Auto.columns(m, 1, 1, 0), 2);
    assert_eq!(Detail::Authored.columns(m, 1, 1, 1), 4);
    assert_eq!(Detail::Width(5).columns(m, 1, 1, 1), 4);
    for (curve, expected) in [(Curve::Linear, 4), (Curve::Exponential, 2), (Curve::Logarithmic, 6)] {
        assert_eq!(Detail::Transition(curve, Direction::Up).columns(m, 1, 20, 20), expected);
        assert_eq!(Detail::Transition(curve, Direction::Down).columns(m, 1, 20, 20), 8 - expected);
    }
    assert_eq!(source.frame(1, 4, true).unwrap().content, "yyyy\nyyyy\nyyyy\nyyyy\n");
}

#[test]
fn tiled_manifest_and_archive_survive_saving_and_reject_damaged_tiles() {
    let root = tempfile::tempdir().unwrap();
    let raw = binary(&[2], 2, false);
    let compressed = gzip(&raw);
    let tile = json!({"url":"width-2.bin.gz","columns":2,"start":0,"count":2,"bytes":raw.len(),"sha256":format!("{:x}",Sha256::digest(&raw))});
    let mut transport = json!({"format":"cascii-tiled-animation","version":1,"framesPerTile":2,"manifest":manifest(&[2],2,false),"tiles":[tile]});
    let path = root.path().join("transport.json.gz");
    fs::write(&path, gzip(&serde_json::to_vec(&transport).unwrap())).unwrap();
    fs::write(root.path().join("width-2.bin.gz"), &compressed).unwrap();
    let library = root.path().join("library");
    let (_, saved) = save_animation(&path, Some("tiles".into()), &library).unwrap();
    assert_eq!(Frames::open(&Input::Path(saved)).unwrap().frame(1, 2, true).unwrap().content, "yy\nyy\n");

    transport["archive"] = json!({"format":"cascii-preload-archive","version":1,"payloadBytes":compressed.len()});
    transport["tiles"][0]["offset"] = json!(0);
    transport["tiles"][0]["length"] = json!(compressed.len());
    let index = gzip(&serde_json::to_vec(&transport).unwrap());
    let size = (index.len() + 10) as u32;
    let mut archive = index[..10].to_vec();
    archive[3] = 4;
    archive.extend_from_slice(&[8, 0, b'C', b'A', 4, 0]);
    archive.extend_from_slice(&size.to_le_bytes());
    archive.extend_from_slice(&index[10..]);
    archive.extend_from_slice(&compressed);
    let path = root.path().join("demo.bin.gz");
    fs::write(&path, &archive).unwrap();
    let (name, saved) = save_animation(&path, None, &library).unwrap();
    assert_eq!(name, "demo");
    assert_eq!(Frames::open(&Input::Path(saved)).unwrap().frame(0, 2, false).unwrap().content, "xx\nxx\n");
    let last = archive.len() - 1;
    archive[last] ^= 1;
    fs::write(&path, &archive).unwrap();
    assert!(save_animation(&path, Some("damaged".into()), &library).is_err());
    assert!(!library.join("damaged").exists());
}

#[test]
fn http_playback_fetches_index_and_only_the_requested_frame() {
    use std::{io::{BufRead, BufReader}, net::TcpListener, thread};
    let bytes = binary(&[2, 4], 3, true);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ranges = Vec::new();
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            let mut range = None;
            while reader.read_line(&mut line).unwrap() != 0 && line != "\r\n" {
                if let Some(value) = line.to_lowercase().strip_prefix("range: bytes=") {
                    let (start, end) = value.trim().split_once('-').unwrap();
                    range = Some((start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap()));
                }
                line.clear();
            }
            let (start, end) = range.unwrap();
            write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len(),end-start+1).unwrap();
            stream.write_all(&bytes[start..=end]).unwrap();
            ranges.push((start, end));
        }
        ranges
    });
    let input = Input::from_path(Path::new(&format!("http://{addr}/frames.bin"))).unwrap();
    let mut frames = Frames::open(&input).unwrap();
    assert_eq!(frames.frame(2, 4, false).unwrap().dimensions(), (4, 4));
    let ranges = server.join().unwrap();
    assert!(ranges[5].0 > ranges[4].1 + 1);
    assert_eq!(ranges[5].1 - ranges[5].0 + 1, 20);
}


#[test]
fn legacy_count_with_json_prefix_and_sibling_metadata_still_plays() {
    let root = tempfile::tempdir().unwrap();
    let mut bytes = 123_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    for _ in 0..123 { bytes.extend_from_slice(&[b'Q', 200, 200, 200]); }
    fs::write(root.path().join("clip.bin"), bytes).unwrap();
    fs::write(root.path().join("metadata.json"), "{}").unwrap();
    let mut frames = Frames::open(&Input::Path(root.path().to_owned())).unwrap();
    assert_eq!(frames.frame(122, 0, true).unwrap().content, "Q\n");
}
