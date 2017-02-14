//! Black-and-white images in a format that's optimized for OCR calculations.

use cast;
use image::{Rgba, RgbaImage};
use std::collections::HashMap;

use errors::*;
#[cfg(test)]
use test_util::{rgba_hex, test_images};

/// Extension methods for `image::Rgba`.
trait RgbaExt {
    /// Is this color transparent or partially transparent?
    fn is_transparent(&self) -> bool;
}

impl RgbaExt for Rgba<u8> {
    fn is_transparent(&self) -> bool {
        self.data[3] < 0xff
    }
}

/// Extension methods for `image::RgbaImage`.
trait RgbaImageExt {
    /// Return the value of the pixel at `x` and `y` if those coordinates
    /// fall inside the image, or `None` if they're out of bounds.
    fn get_opt(&self, x: i32, y: i32) -> Option<&Rgba<u8>>;
}

impl RgbaImageExt for RgbaImage {
    fn get_opt(&self, x: i32, y: i32) -> Option<&Rgba<u8>> {
        if x < 0 || y < 0 {
            return None;
        }
        // It's safe to assert here because we just checked above.
        let x = cast::u32(x).expect("x should be in bounds");
        let y = cast::u32(y).expect("y should be in bounds");
        if x >= self.width() || y >= self.height() {
            None
        } else {
            Some(self.get_pixel(x, y))
        }
    }
}

/// Different kinds of colors we might find in an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorType {
    /// This color is transparent.
    Transparent,
    /// This color appears to be a shadow color which we should treat as
    /// transparent to facilitate letter separation.
    Shadow,
    /// This color is opaque, and should be used for letter recognition.
    Opaque,
}

impl From<ColorType> for usize {
    /// Map `ColorType` to something we can use to index an array.
    fn from(ct: ColorType) -> usize {
        match ct {
            ColorType::Transparent => 0,
            ColorType::Shadow => 1,
            ColorType::Opaque => 2,
        }
    }
}

/// Information about the `ColorType` of pixels adjacent to a given color.
#[derive(Debug)]
struct AdjacentPixelInfo {
    data: [usize; 3],
}

impl AdjacentPixelInfo {
    fn new() -> AdjacentPixelInfo {
        AdjacentPixelInfo { data: [0; 3] }
    }

    fn count(&self, ct: ColorType) -> usize {
        self.data[usize::from(ct)]
    }

    fn incr_count(&mut self, ct: ColorType) {
        self.data[usize::from(ct)] += 1;
    }

    fn total(&self) -> usize {
        self.data[0] + self.data[1] + self.data[2]
    }

    fn fraction_adj_to(&self, ct: ColorType) -> f64 {
        cast::f64(self.count(ct)) / cast::f64(self.total())
    }

    fn looks_like_opaque_inside_shadow(&self) -> bool {
        self.fraction_adj_to(ColorType::Opaque) > 0.95
    }

    fn looks_like_shadow(&self) -> bool {
        self.fraction_adj_to(ColorType::Opaque) > 0.33 &&
            self.fraction_adj_to(ColorType::Transparent) > 0.33
    }
}

/// Classify the colors in an image as transparent or non-transparent.
fn classify_colors(image: &RgbaImage) -> Result<HashMap<Rgba<u8>, ColorType>> {
    // First divide colors into transparent and opaque based on alpha.
    let mut classification = HashMap::new();
    for px in image.pixels() {
        if px.is_transparent() {
            classification.entry(*px).or_insert(ColorType::Transparent);
        } else {
            classification.entry(*px).or_insert(ColorType::Opaque);
        }
    }
    debug!("color classification (initial): {:?}", &classification);

    // Calculate which colors are adjacent to which color classifications.
    // We'll use this to detect "shadow" colors.
    let mut adjacent = HashMap::new();
    for c in classification.keys() {
        if !c.is_transparent() {
            adjacent.insert(*c, AdjacentPixelInfo::new());
        }
    }
    for (x, y, px) in image.enumerate_pixels() {
        // Don't compute adjacency info for transparent pxiels.
        if px.is_transparent() {
            continue;
        }
        // Look at the 3x3 grid around this pixel.
        for &dy in &[-1, 0, 1] {
            for &dx in &[-1, 0, 1] {
                // Don't compare to ourselves.
                if dx == 0 && dy == 0 {
                    continue;
                }

                // Get our neighboring pixel, if it's in bounds.
                let px_adj_opt =
                    image.get_opt(cast::i32(x)? + dx, cast::i32(y)? + dy);

                // Don't count pixels of the same color.
                if px_adj_opt == Some(px) {
                    continue;
                }

                // Classify our neighboring color an increment our counter.
                let ct_adj = px_adj_opt
                    .map_or_else(|| ColorType::Transparent,
                                 |px_adj| {
                                     *classification.get(px_adj)
                                         .expect("unknown classification")
                                 });
                adjacent.get_mut(px)
                    .expect("unknown adjacent color")
                    .incr_count(ct_adj);
            }
        }
    }
    debug!("color adjacency: {:?}", &adjacent);

    // Check to see whether we have any opaque colors which seem to be
    // _surrounded_ by a shadow.
    let total_adj: usize = adjacent.values().map(|adj| adj.total()).sum();
    let mut have_opaque_inside_shadow = false;
    for adj in adjacent.values() {
        // Only check colors which make up a reasonable fraction of our
        // total.
        if adj.total() >= total_adj / 4 && adj.looks_like_opaque_inside_shadow() {
            have_opaque_inside_shadow = true;
        }
    }

    // If we have colors _surrounded_ by shadows, look for the shadow
    // colors.
    if have_opaque_inside_shadow {
        for (c, adj) in adjacent.iter() {
            if adj.looks_like_shadow() {
                classification.insert(c.to_owned(), ColorType::Shadow);
            }
        }
    }

    debug!("color classification (final): {:?}", &classification);
    Ok(classification)
}

#[test]
fn classify_colors_as_transparent_and_opaque() {
    //use env_logger;
    //env_logger::init().unwrap();

    let images = test_images().unwrap();
    let colors = classify_colors(&images[0]).unwrap();
    assert_eq!(colors.len(), 4);
    assert_eq!(*colors.get(&rgba_hex(0x00000000)).unwrap(), ColorType::Transparent);
    assert_eq!(*colors.get(&rgba_hex(0x000000ff)).unwrap(), ColorType::Shadow);
    assert_eq!(*colors.get(&rgba_hex(0x999999ff)).unwrap(), ColorType::Opaque);
    assert_eq!(*colors.get(&rgba_hex(0xf0f0f0ff)).unwrap(), ColorType::Opaque);
}
