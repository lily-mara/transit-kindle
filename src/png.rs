use crate::{
    config::ConfigFile,
    layout::{Agency, Layout},
};
use eyre::{bail, eyre, Result};
use skia_safe::{
    Bitmap, Canvas, Color4f, Font, FontStyle, ImageInfo, Paint, Point, TextBlob, Typeface,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RenderTarget {
    Kindle,
    Browser,
}

fn render_ctx(
    render_target: RenderTarget,
    config_file: &ConfigFile,
    closure: impl FnOnce(&mut Canvas) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut bitmap = Bitmap::new();
    if !bitmap.set_info(
        &ImageInfo::new(
            (config_file.layout.width, config_file.layout.height),
            skia_safe::ColorType::Gray8,
            skia_safe::AlphaType::Unknown,
            None,
        ),
        None,
    ) {
        bail!("failed to initialize skia bitmap");
    }
    bitmap.alloc_pixels();

    let mut canvas =
        Canvas::from_bitmap(&bitmap, None).ok_or(eyre!("failed to construct skia canvas"))?;

    canvas.clear(Color4f::new(1.0, 1.0, 1.0, 1.0));

    closure(&mut canvas)?;

    let image = bitmap.as_image();

    let final_image = if render_target == RenderTarget::Kindle {
        let mut rotated_bitmap = Bitmap::new();
        if !rotated_bitmap.set_info(
            &ImageInfo::new(
                (config_file.layout.height, config_file.layout.width),
                skia_safe::ColorType::Gray8,
                skia_safe::AlphaType::Unknown,
                None,
            ),
            None,
        ) {
            bail!("failed to initialize skia bitmap");
        }
        rotated_bitmap.alloc_pixels();

        let mut rotated_canvas = Canvas::from_bitmap(&rotated_bitmap, None)
            .ok_or(eyre!("failed to construct skia canvas"))?;

        rotated_canvas.translate(Point::new(config_file.layout.height as f32, 0.0));
        rotated_canvas.rotate(90.0, Some(Point::new(0.0, 0.0)));
        rotated_canvas.draw_image(image, (0, 0), None);

        rotated_bitmap.as_image()
    } else {
        image
    };

    let image_data = final_image
        .encode(None, skia_safe::EncodedImageFormat::PNG, None)
        .ok_or(eyre!("failed to encode skia image"))?;

    Ok(image_data.as_bytes().into())
}

pub fn stops_png(
    render_target: RenderTarget,
    layout: Layout,
    config_file: &ConfigFile,
) -> Result<Vec<u8>> {
    let black_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    let grey_paint = Paint::new(Color4f::new(0.6, 0.6, 0.6, 1.0), None);

    let typeface = Typeface::new("Liberation Sans", FontStyle::bold())
        .ok_or(eyre!("failed to construct skia typeface"))?;

    let font = Font::new(typeface, 24.0);

    let draw_data =
        |canvas: &mut Canvas, agency: &Agency, (x1, x2): (i32, i32), y: &mut i32| -> Result<()> {
            if x1 > 0 {
                canvas.draw_line((x1, 0), (x1, config_file.layout.height), &black_paint);
            }

            let lines_len = agency.lines.len();

            for (idx, line) in agency.lines.iter().enumerate() {
                let x = x1 + 20;

                let line_id_blob = TextBlob::new(&line.id, &font)
                    .ok_or(eyre!("failed to construct skia text blob"))?;

                let line_id_bounds = line_id_blob.bounds();

                let line_id_oval = line_id_bounds.with_offset((x, *y));

                canvas.draw_oval(line_id_oval, &grey_paint);

                canvas.draw_text_blob(&line_id_blob, (x, *y), &black_paint);

                let destination_blob = TextBlob::new(&line.destination, &font)
                    .ok_or(eyre!("failed to construct skia text blob"))?;
                canvas.draw_text_blob(
                    destination_blob,
                    ((x + line_id_bounds.width() as i32), *y),
                    &black_paint,
                );

                let mins = line.departure_minutes_str();
                let time_text = format!("{mins} min");

                let time_blob = TextBlob::new(time_text, &font)
                    .ok_or(eyre!("failed to construct skia text blob"))?;

                let x = x2 - time_blob.bounds().width() as i32;
                canvas.draw_text_blob(time_blob, (x, *y), &black_paint);

                if idx < (lines_len - 1) {
                    canvas.draw_line((x1 + 40, *y + 15), (x2 - 40, *y + 15), &grey_paint);
                    *y += 40;
                } else {
                    *y += 15;
                }
            }

            canvas.draw_line((x1, *y), (x2, *y), &black_paint);
            *y += 28;

            Ok(())
        };

    let halfway = config_file.layout.width / 2;

    let image_data = render_ctx(render_target, config_file, |canvas| {
        let mut y = 38;
        for agency in &layout.left.agencies {
            draw_data(canvas, agency, (0, halfway), &mut y)?;
        }

        let mut y = 38;
        for agency in &layout.right.agencies {
            draw_data(canvas, agency, (halfway, config_file.layout.width), &mut y)?;
        }

        Ok(())
    })?;

    Ok(image_data)
}

pub fn error_png(
    render_target: RenderTarget,
    config_file: &ConfigFile,
    error: eyre::Report,
) -> Result<Vec<u8>> {
    let black_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);

    let typeface = Typeface::new("Liberation Sans", FontStyle::normal())
        .ok_or(eyre!("failed to construct skia typeface"))?;

    let big_font = Font::new(&typeface, 36.0);
    let small_font: skia_safe::Handle<_> = Font::new(typeface, 12.0);

    let failure_blob =
        TextBlob::new("ERROR", &big_font).ok_or(eyre!("failed to construct skia text blob"))?;

    let data = render_ctx(render_target, config_file, move |canvas| {
        canvas.draw_text_blob(failure_blob, (100, 200), &black_paint);
        let mut y = 250;
        for e in error.chain() {
            let error_blob = TextBlob::new(format!("{e}"), &small_font)
                .ok_or(eyre!("failed to construct skia text blob"))?;
            canvas.draw_text_blob(error_blob, (100, y), &black_paint);
            y += 20;
        }
        Ok(())
    })?;

    Ok(data)
}
