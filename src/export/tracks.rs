//! Output a video as a series of short audio tracks, short enough to make
//! it easy to skip backwards a conversation with most MP3 players.

use std::default::Default;
use std::io::{Cursor, Write};

use errors::*;
use export::Exporter;
use time::{Period, seconds_to_hhmmss};
use video::Id3Metadata;

// Truncate a string to fit within the specified number of Unicode
// characters.
fn truncate(max: usize, s: &str) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut result: String = s.chars().take(max-1).collect();
        result.push_str("…");
        result
    }
}

// Should we merge two time periods into one?
fn should_merge(p1: Period, p2: Period) -> bool {
    if p1.duration() >= 30.0 {
        false
    } else {
        match p1.distance(p2) {
            None => true,
            Some(d) if d <= 5.0 => true,
            _ => false
        }
    }
}

// A conversation.
#[derive(Debug)]
struct Conv {
    period: Period,
    text: String,
}

/// Export the video as a set of tracks.
pub fn export_tracks(exporter: &mut Exporter) -> Result<()> {
    let mut convs: Vec<Conv> = vec!();

    // Figure out how to combine subtitles into conversations.
    for sub in &exporter.foreign().subtitles.subtitles {
        if let Some(prev) = convs.last_mut() {
            if should_merge(prev.period, sub.period) {
                prev.period = prev.period.union(sub.period);
                prev.text.push_str("\n");
                prev.text.push_str(&sub.plain_text());
                continue;
            }
        }
        convs.push(Conv { period: sub.period, text: sub.plain_text() });
    }

    // Add padding to each audio clip.
    for conv in convs.iter_mut() {
        conv.period = conv.period.grow(2.5, 2.5);
    }

    // Turn overlapping clips into seamless transitions.
    // TODO: Fade in/out when not a seamless transition.
    for i in 1..convs.len() {
        if convs[i].period.begin() < convs[i-1].period.end()  {
            let overlap = Period::new(convs[i].period.begin(),
                                      convs[i-1].period.end())
                .unwrap();
            let midpoint = overlap.midpoint();
            debug!("Overlap: {:?} {:?} {:?} {}",
                   &convs[i-1], &convs[i], overlap, midpoint);
            convs[i-1].period =
                Period::new(convs[i-1].period.begin(), midpoint).unwrap();
            convs[i].period =
                Period::new(midpoint, convs[i].period.end()).unwrap();
            assert!(convs[i].period.begin() >= convs[i-1].period.end());
        }
    }

    // Schedule exports and write our our m3u8 playlist (like m3u, but
    // UTF-8).
    //
    // TODO: Genre, artist, album, track title, track number.
    let end_at = exporter.end_at();
    let foreign_lang = exporter.foreign().language;
    let mut playlist = Cursor::new(vec!());
    for (i, conv) in convs.iter().enumerate() {
        debug!("Conv: {:7.1} -> {:7.1} for {:7.1}",
               conv.period.begin(), conv.period.end(), conv.period.duration());

        match end_at {
            Some(seconds) if conv.period.begin() > seconds => {
                println!("Ending at {:?} seconds of video.", seconds);
                break;
            }
            _ => {}
        }

        // Build our track name.
        let name = format!("{} {}", seconds_to_hhmmss(conv.period.begin()),
                           truncate(32, &conv.text).replace("\n", " "));

        // Compute our metadata.
        let metadata = Id3Metadata {
            genre: Some("substudy".to_owned()),
            album: Some(exporter.file_stem().to_owned()),
            track_number: Some((i+1, convs.len())),
            track_name: Some(name),
            lyrics: Some(conv.text.clone()),
            ..Default::default()
        };

        // Export as an audio file, and record the path in our playlist.
        let path =
            exporter.schedule_audio_export_ext(foreign_lang, conv.period,
                                               metadata);
        try!(writeln!(playlist, "{}", &path));
    }
    try!(exporter.export_data_file("playlist.m3u8", &playlist.get_ref()));

    // Extract our media files.
    try!(exporter.finish_exports());

    Ok(())
}
