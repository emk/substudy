//! Export to web-page based "review" format.

use handlebars::Handlebars;
use rustc_serialize::json::{ToJson, Json};
use std::collections::BTreeMap;
use std::str::from_utf8;

use errors::*;
use export::Exporter;
use lang::Lang;
use time::Period;

/// Information about a subtitle for use by our Handlebars HTML template.
/// This needs to implement `ToJson`, because that's what Handlebars wants
/// as input.
struct SubtitleInfo {
    index: usize,
    image_path: String,
    audio_path: String,
    foreign_text: Option<String>,
    native_text: Option<String>,
}

impl ToJson for SubtitleInfo {
    fn to_json(&self) -> Json {
        let mut m: BTreeMap<String, Json> = BTreeMap::new();
        m.insert("index".to_string(), self.index.to_json());
        m.insert("image_path".to_string(), self.image_path.to_json());
        m.insert("audio_path".to_string(), self.audio_path.to_json());
        m.insert("foreign_text".to_string(), self.foreign_text.to_json());
        m.insert("native_text".to_string(), self.native_text.to_json());
        m.to_json()
    }
}

/// Export-related information for use by our Handlebars HTML template.
struct ExportInfo {
    filename: String,
    subtitles: Vec<SubtitleInfo>,
    foreign_lang: Option<Lang>,
    native_lang: Option<Lang>,
}

impl ToJson for ExportInfo {
    fn to_json(&self) -> Json {
        let mut m: BTreeMap<String, Json> = BTreeMap::new();
        m.insert("filename".to_string(), self.filename.to_json());
        m.insert("subtitles".to_string(), self.subtitles.to_json());
        m.insert("foreign_lang".to_string(), self.foreign_lang.to_json());
        m.insert("native_lang".to_string(), self.native_lang.to_json());
        m.to_json()
    }
}

/// Export the video and subtitles as a web page in "reviewable" format.
pub fn export_review(exporter: &mut Exporter) -> Result<()> {
    let end_at = exporter.end_at();
    let foreign_lang = exporter.foreign().language;
    // Start preparing information we'll pass to our HTML template.
    let mut bindings = ExportInfo {
        filename: exporter.title().to_owned(),
        subtitles: vec!(),
        foreign_lang: foreign_lang,
        native_lang: exporter.native().and_then(|n| n.language)
    };

    // Align our input files and iterate.
    let aligned = exporter.align();
    for sub in aligned.iter().enumerate() {
        let (i, &(ref foreign, ref native)) = sub;
        let index = i + 1;
        let period = Period::from_union_opt(
            foreign.as_ref().map(|s| s.period),
            native.as_ref().map(|s| s.period),
        ).expect("subtitle pair must not be empty").grow(0.5, 0.5);

        match end_at {
            Some(seconds) if period.begin() > seconds => {
                println!("Ending at {:?} seconds of video.", seconds);
                break;
            }
            _ => {}
        }

        let image_path = exporter.schedule_image_export(period.midpoint());
        let audio_path = exporter.schedule_audio_export(foreign_lang, period);

        bindings.subtitles.push(SubtitleInfo {
            index: index,
            image_path: image_path,
            audio_path: audio_path,
            foreign_text: foreign.as_ref().map(|s| s.plain_text()),
            native_text: native.as_ref().map(|s| s.plain_text()),
        });
    }

    // Write out our support files.
    try!(exporter.export_data_file("style.css", include_bytes!("style.css")));
    try!(exporter.export_data_file("play.svg", include_bytes!("play.svg")));

    // Render and write out our HTML.
    let template = try!(from_utf8(include_bytes!("review.html.hbs")));
    let mut handlebars = Handlebars::new();
    try!(handlebars.register_template_string("review", template.to_owned()));
    let html = try!(handlebars.render("review", &bindings));
    try!(exporter.export_data_file("index.html", html.as_bytes()));

    // Extract our media files.
    try!(exporter.finish_exports());

    Ok(())
}
