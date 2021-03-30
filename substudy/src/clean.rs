//! Tools for cleaning up subtitle files and getting them into some
//! kind of normalized format.

use common_failures::prelude::*;
use regex::Regex;
use srt::{Subtitle, SubtitleFile};
use std::borrow::Cow;

/// Remove the formatting from a subtitle.
pub fn strip_formatting(line: &str) -> Cow<str> {
    let formatting = Regex::new(r"<[a-z/][^>]*>").unwrap();
    formatting.replace_all(&line, "")
}

// Clean up a single subtitle line.
fn clean_line(line: &str) -> String {
    // Used to remove the following common sorts of closed-caption clutter:
    //
    //     SPEAKER:
    //     ( sound effect )
    //     ♪ music ♪
    //     [Music]
    let clutter = Regex::new(r"(\([^)]*\)|\[[^\]]*\]|♪[^♪]*♪|[A-Z]{2,} ?:)").unwrap();

    // Used to compress and normalize consecutive whitespace.
    let whitespace = Regex::new(r"\s+").unwrap();

    whitespace
        .replace_all(&clutter.replace_all(line, ""), " ")
        .trim()
        .to_string()
}

// Clean up a subtitle, or discard it if it looks useless.
fn clean_subtitle(sub: &Subtitle) -> Option<Subtitle> {
    let lines: Vec<String> = sub.lines
        .iter()
        .map(|l| clean_line(&l))
        .filter(|l| l.len() > 0)
        .map(|l| l.to_string())
        .collect();
    if lines.len() == 0 {
        return None;
    }
    Some(Subtitle {
        index: sub.index,
        period: sub.period,
        lines: lines,
    })
}

/// Clean up various issues with subtitle files, including:
///
/// * Out of order subtitles.
/// * Overlapping subtitles.
/// * Sound effects.
/// * Music symbols.
pub fn clean_subtitle_file(file: &SubtitleFile) -> Result<SubtitleFile> {
    // Clean individual subtitles and sort.
    let mut subs: Vec<Subtitle> =
        file.subtitles.iter().filter_map(clean_subtitle).collect();
    subs.sort_by(|a, b| {
        a.period.begin().partial_cmp(&b.period.begin()).unwrap()
    });

    // Fix overlaps.
    if subs.len() >= 2 {
        for i in 0..subs.len() - 1 {
            let limit = subs[i + 1].period.begin();
            subs[i].period.end_before(limit)?;
        }
    }

    // Renumber and return.
    for (i, ref mut sub) in subs.iter_mut().enumerate() {
        sub.index = i + 1;
    }
    Ok(SubtitleFile { subtitles: subs })
}

#[test]
fn test_clean_subtitle_file() {
    let dirty = SubtitleFile::from_str(
        r"19
00:01:03,163 --> 00:01:04,664
They've arrived.
( <i>door slams</i> )

20
00:01:07,700 --> 00:01:10,736
( <i>cheering</i> )

21
00:01:12,839 --> 00:01:13,840
♪ ♪

22
00:01:14,432 --> 00:01:16,637
[Music]

18
00:01:02,328 --> 00:01:03,162
JOE: Hey! ( waves arms )

53
00:02:47,965 --> 00:02:50,684
Out of order.

52
00:02:42,658 --> 00:02:48,865
Overlapping.
",
    ).unwrap();

    let cleaned = "\u{FEFF}1
00:01:02,328 --> 00:01:03,162
Hey!

2
00:01:03,163 --> 00:01:04,664
They've arrived.

3
00:02:42,658 --> 00:02:47,964
Overlapping.

4
00:02:47,965 --> 00:02:50,684
Out of order.
";

    assert_eq!(cleaned, &clean_subtitle_file(&dirty).unwrap().to_string());
}
