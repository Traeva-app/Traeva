//! The metadata-free JPEG and base64, on made-up bytes: no decoder runs.

use super::*;

/// One segment: marker, length, body.
fn segment(code: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, code];
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend(body);
    out
}

fn plain_jfif() -> Vec<u8> {
    segment(0xE0, b"JFIF\0\x01\x01\x00\x00\x48\x00\x48\x00\x00")
}

/// The tables, the frame header and the scan of a made-up JPEG, with a
/// stuffed byte and a restart marker in its data, up to the end marker.
fn picture() -> Vec<u8> {
    let mut out = segment(0xDB, &[0; 65]);
    out.extend(segment(0xC0, &[8, 0, 16, 0, 32, 1, 1, 0x11, 0]));
    out.extend(segment(0xC4, &[0; 20]));
    out.extend(segment(0xDD, &[0, 4]));
    out.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
    out.extend([0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
    out.extend(END);
    out
}

/// The made-up JPEG with ImageIO's two segments, an XMP packet, an ICC
/// profile, a comment and a JFIF thumbnail before its tables.
fn with_metadata() -> Vec<u8> {
    let mut out = START.to_vec();
    out.extend(plain_jfif());
    out.extend(segment(0xE1, b"Exif\0\0MM\0*FAKE-CAMERA-MAKE GPS 0,0"));
    out.extend(segment(
        0xE1,
        b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>XMP</x:xmpmeta>",
    ));
    out.extend(segment(0xE2, b"ICC_PROFILE\0\x01\x01sRGB"));
    out.extend(segment(0xED, b"Photoshop 3.0\08BIM\x04\x04"));
    out.extend(segment(0xFE, b"FAKE-ARTIST-NAME"));
    out.extend(segment(0xE0, b"JFXX\0\x10thumbnail"));
    out.extend(picture());
    out
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn the_metadata_segments_are_taken_out_and_the_picture_kept() {
    let bare = bare_jpeg(&with_metadata()).unwrap();
    let expected = [START.to_vec(), plain_jfif(), picture()].concat();
    assert_eq!(bare, expected);
    for word in [
        &b"Exif"[..],
        b"GPS",
        b"FAKE",
        b"XMP",
        b"ICC_PROFILE",
        b"8BIM",
        b"JFXX",
    ] {
        assert!(!contains(&bare, word), "{}", String::from_utf8_lossy(word));
    }
    assert!(is_bare_jpeg(&bare));
    assert!(!is_bare_jpeg(&with_metadata()));
    // Without a JFIF header at all, also bare.
    let no_jfif = [START.to_vec(), picture()].concat();
    assert_eq!(bare_jpeg(&no_jfif).as_deref(), Some(&no_jfif[..]));
}

#[test]
fn a_jfif_header_with_a_thumbnail_is_taken_out() {
    let thumbnail = segment(
        0xE0,
        &[
            b"JFIF\0\x01\x01\x00\x00\x48\x00\x48\x01\x01".as_slice(),
            &[0, 0, 0],
        ]
        .concat(),
    );
    let jpeg = [START.to_vec(), thumbnail, picture()].concat();
    assert_eq!(
        bare_jpeg(&jpeg).unwrap(),
        [START.to_vec(), picture()].concat()
    );
}

#[test]
fn only_a_whole_single_scan_jpeg_is_accepted() {
    let good = [START.to_vec(), picture()].concat();
    let cut = |n: usize| good[..good.len() - n].to_vec();
    let mut segment_in_the_data = good[..good.len() - 2].to_vec();
    segment_in_the_data.extend(segment(0xE1, b"Exif\0\0"));
    segment_in_the_data.extend(END);
    let mut second_scan = good[..good.len() - 2].to_vec();
    second_scan.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
    second_scan.extend(END);
    let bad_length = [START.to_vec(), vec![0xFF, 0xDB, 0x00, 0x01], picture()].concat();
    let past_the_end = [START.to_vec(), vec![0xFF, 0xE1, 0xFF, 0xFF, 0x00]].concat();
    let fill_byte = [START.to_vec(), vec![0xFF], picture()].concat();
    let restart_before_scan = [START.to_vec(), vec![0xFF, 0xD0], picture()].concat();
    for bad in [
        Vec::new(),
        b"\x89PNG\r\n\x1a\n".to_vec(),
        START.to_vec(),
        cut(1),
        cut(2),
        [good.clone(), vec![0]].concat(),
        [good.clone(), END.to_vec()].concat(),
        segment_in_the_data,
        second_scan,
        bad_length,
        past_the_end,
        fill_byte,
        restart_before_scan,
        [START.to_vec(), START.to_vec(), picture()].concat(),
    ] {
        assert!(bare_jpeg(&bad).is_none(), "{bad:x?}");
        assert!(!is_bare_jpeg(&bad));
    }
}

#[test]
fn base64_is_the_standard_alphabet_with_padding() {
    for (bytes, text) in [
        (&b""[..], ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"fooba", "Zm9vYmE="),
        (b"foobar", "Zm9vYmFy"),
        (&[0xFF, 0xD8, 0xFF, 0xFE], "/9j//g=="),
    ] {
        assert_eq!(base64_encode(bytes), text);
        assert_eq!(base64_decode(text).as_deref(), Some(bytes));
    }
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(base64_decode(&base64_encode(&all)), Some(all));
}

#[test]
fn base64_that_is_not_exactly_the_encoding_is_refused() {
    for text in [
        "Zg", "Zg=", "Zg===", "====", "Z===", "Zm=v", "Zh==", "Zm9", "Zm9v\n", " Zm9v", "Zm9-",
        "Zm9_", "Zm9v====", "Zg==Zm9v",
    ] {
        assert!(base64_decode(text).is_none(), "{text:?}");
    }
}
