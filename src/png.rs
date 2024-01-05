use crate::{
    config::ConfigFile,
    layout::{Agency, Layout, Row},
};
use chrono::prelude::*;
use eyre::{bail, eyre, Result};
use skia_safe::{
    utils::text_utils::Align, Bitmap, Canvas, Color4f, Font, FontStyle, ImageInfo, Paint, Point,
    Rect, TextBlob, Typeface,
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
    let mut black_paint_heavy = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    black_paint_heavy.set_stroke_width(2.0);

    let grey_paint = Paint::new(Color4f::new(0.7, 0.7, 0.7, 1.0), None);
    let light_grey_paint = Paint::new(Color4f::new(0.8, 0.8, 0.8, 1.0), None);

    let typeface = Typeface::new("Liberation Sans", FontStyle::bold())
        .ok_or(eyre!("failed to construct skia typeface"))?;

    let font = Font::new(typeface, 24.0);

    let draw_text_in_bubble = |canvas: &mut Canvas, text: &str, x: i32, y: i32| -> Result<Rect> {
        let blob = TextBlob::new(text, &font).ok_or(eyre!("failed to construct skia text blob"))?;

        let mut bounds = *blob.bounds();
        bounds.set_xywh(
            bounds.x() + 1.0,
            bounds.y(),
            bounds.width() - 5.0,
            bounds.height(),
        );

        let rect = bounds.with_offset((x, y));

        canvas.draw_round_rect(rect, 10.0, 10.0, &grey_paint);

        canvas.draw_text_blob(&blob, (x, y), &black_paint);

        Ok(bounds)
    };

    let draw_agency_row =
        |canvas: &mut Canvas, agency: &Agency, (x1, x2): (i32, i32), y: &mut i32| -> Result<()> {
            *y += 4;

            let lines_len = agency.lines.len();

            for (idx, line) in agency.lines.iter().enumerate() {
                let x = x1 + 20;

                let line_id_bounds = draw_text_in_bubble(canvas, &line.id, x, *y)?;

                let destination_blob = TextBlob::new(&line.destination, &font)
                    .ok_or(eyre!("failed to construct skia text blob"))?;
                canvas.draw_text_blob(
                    destination_blob,
                    ((x + line_id_bounds.width() as i32), *y),
                    &black_paint,
                );

                let mins = line.departure_minutes_str();
                let time_text = format!("{mins} min");

                canvas.draw_str_align(
                    time_text,
                    (x2 as f32 - 20.0, (*y) as f32),
                    &font,
                    &black_paint,
                    Align::Right,
                );

                if idx < (lines_len - 1) {
                    canvas.draw_line((x1 + 40, *y + 15), (x2 - 40, *y + 15), &grey_paint);
                    *y += 48;
                } else {
                    *y += 15;
                }
            }

            Ok(())
        };

    let draw_text_row =
        |canvas: &mut Canvas, text: &str, (x1, x2): (i32, i32), y: &mut i32| -> Result<()> {
            canvas.draw_rect(
                Rect::new(x1 as f32, *y as f32, x2 as f32, (*y + 40) as f32),
                &light_grey_paint,
            );
            *y += 28;

            canvas.draw_str_align(
                text,
                ((x1 + x2) as f32 / 2.0, *y as f32),
                &font,
                &black_paint,
                Align::Center,
            );

            *y += 12;

            Ok(())
        };

    let draw_row =
        |canvas: &mut Canvas, row: &Row, (x1, x2): (i32, i32), y: &mut i32| -> Result<()> {
            if *y > 0 {
                canvas.draw_line((x1, *y), (x2, *y), &black_paint_heavy);
                *y += 28;
            }

            match row {
                Row::Agency(agency) => draw_agency_row(canvas, agency, (x1, x2), y)?,
                Row::Text(text) => draw_text_row(canvas, text, (x1, x2), y)?,
            }

            Ok(())
        };

    let halfway = config_file.layout.width / 2;

    let draw_footer = |canvas: &mut Canvas| {
        let bottom_box_y = (config_file.layout.height - 40) as f32;

        canvas.draw_line(
            (0 as f32, bottom_box_y),
            (config_file.layout.width as f32, bottom_box_y),
            &black_paint_heavy,
        );

        canvas.draw_rect(
            Rect::new(
                0.0,
                bottom_box_y,
                config_file.layout.width as f32,
                config_file.layout.height as f32,
            ),
            &light_grey_paint,
        );

        let now = Local::now();
        let time = now.format("%a %b %d - %H:%M").to_string();

        canvas.draw_str_align(
            time,
            (halfway as f32, (config_file.layout.height - 10) as f32),
            &font,
            &black_paint,
            Align::Center,
        );
    };

    let image_data = render_ctx(render_target, config_file, |canvas| {
        let mut y = 0;
        for row in &layout.left.rows {
            draw_row(canvas, row, (0, halfway), &mut y)?;
        }

        let mut y = 0;
        for row in &layout.right.rows {
            draw_row(canvas, row, (halfway, config_file.layout.width), &mut y)?;
        }

        canvas.draw_line(
            (halfway, 0),
            (halfway, config_file.layout.height),
            &black_paint_heavy,
        );

        draw_footer(canvas);

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
