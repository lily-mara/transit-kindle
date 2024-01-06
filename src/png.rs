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

struct Render<'a> {
    black_paint: Paint,
    black_paint_heavy: Paint,
    grey_paint: Paint,
    light_grey_paint: Paint,

    typeface: Typeface,
    font: Font,

    canvas: &'a mut Canvas,

    width: f32,
    height: f32,
    y: f32,

    x_midpoint: f32,
}

impl<'a> Render<'a> {
    fn new(canvas: &'a mut Canvas, config: &ConfigFile) -> Result<Self> {
        let mut black_paint_heavy = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
        black_paint_heavy.set_stroke_width(2.0);

        let typeface = Typeface::new("Liberation Sans", FontStyle::bold())
            .ok_or(eyre!("failed to construct skia typeface"))?;

        Ok(Self {
            canvas,

            black_paint: Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None),
            black_paint_heavy,

            grey_paint: Paint::new(Color4f::new(0.7, 0.7, 0.7, 1.0), None),
            light_grey_paint: Paint::new(Color4f::new(0.8, 0.8, 0.8, 1.0), None),

            font: Font::new(&typeface, 24.0),
            typeface,

            width: config.layout.width as f32,
            height: config.layout.height as f32,
            y: 0.0,

            x_midpoint: config.layout.width as f32 / 2.0,
        })
    }

    fn draw_row(&mut self, row: &Row, x1: f32, x2: f32) -> Result<()> {
        if self.y > 0.0 {
            self.canvas
                .draw_line((x1, self.y), (x2, self.y), &self.black_paint_heavy);
            self.y += 28.0;
        }

        match row {
            Row::Agency(agency) => self.draw_agency_row(agency, x1, x2)?,
            Row::Text(text) => self.draw_text_row(text, x1, x2),
        }

        Ok(())
    }

    fn draw_agency_row(&mut self, agency: &Agency, x1: f32, x2: f32) -> Result<()> {
        self.y += 4.0;

        let lines_len = agency.lines.len();

        for (idx, line) in agency.lines.iter().enumerate() {
            let x = x1 + 20.0;

            let line_id_bounds = self.draw_text_in_bubble(&line.id, x)?;

            let destination_blob = TextBlob::new(&line.destination, &self.font)
                .ok_or(eyre!("failed to construct skia text blob"))?;
            self.canvas.draw_text_blob(
                destination_blob,
                (x + line_id_bounds.width(), self.y),
                &self.black_paint,
            );

            let mins = line.departure_minutes_str();
            let time_text = format!("{mins} min");

            self.canvas.draw_str_align(
                time_text,
                (x2 - 20.0, self.y),
                &self.font,
                &self.black_paint,
                Align::Right,
            );

            if idx < (lines_len - 1) {
                self.canvas.draw_line(
                    (x1 + 40.0, self.y + 15.0),
                    (x2 - 40.0, self.y + 15.0),
                    &self.grey_paint,
                );
                self.y += 48.0;
            } else {
                self.y += 15.0;
            }
        }

        Ok(())
    }

    fn draw_text_in_bubble(&mut self, text: &str, x: f32) -> Result<Rect> {
        let blob =
            TextBlob::new(text, &self.font).ok_or(eyre!("failed to construct skia text blob"))?;

        let mut bounds = *blob.bounds();
        bounds.set_xywh(
            bounds.x() + 1.0,
            bounds.y(),
            bounds.width() - 5.0,
            bounds.height(),
        );

        let rect = bounds.with_offset((x, self.y));

        self.canvas
            .draw_round_rect(rect, 10.0, 10.0, &self.grey_paint);

        self.canvas
            .draw_text_blob(&blob, (x, self.y), &self.black_paint);

        Ok(bounds)
    }

    fn draw_footer(&mut self) {
        let bottom_box_y = self.height - 40.0;

        self.canvas.draw_line(
            (0.0, bottom_box_y),
            (self.width, bottom_box_y),
            &self.black_paint_heavy,
        );

        self.canvas.draw_rect(
            Rect::new(0.0, bottom_box_y, self.width, self.height),
            &self.light_grey_paint,
        );

        let now = Local::now();
        let time = now.format("%a %b %d - %H:%M").to_string();

        self.canvas.draw_str_align(
            time,
            (self.x_midpoint, self.height - 10.0),
            &self.font,
            &self.black_paint,
            Align::Center,
        );
    }

    fn draw_text_row(&mut self, text: &str, x1: f32, x2: f32) {
        self.canvas.draw_rect(
            Rect::new(x1, self.y, x2, self.y + 40.0),
            &self.light_grey_paint,
        );
        self.y += 28.0;

        self.canvas.draw_str_align(
            text,
            ((x1 + x2) / 2.0, self.y),
            &self.font,
            &self.black_paint,
            Align::Center,
        );

        self.y += 12.0;
    }

    fn draw(mut self, layout: &Layout) -> Result<()> {
        self.y = 0.0;
        for row in &layout.left.rows {
            self.draw_row(row, 0.0, self.x_midpoint)?;
        }

        self.y = 0.0;
        for row in &layout.right.rows {
            self.draw_row(row, self.x_midpoint, self.width)?;
        }

        self.canvas.draw_line(
            (self.x_midpoint, 0.0),
            (self.x_midpoint, self.height),
            &self.black_paint_heavy,
        );

        self.draw_footer();

        Ok(())
    }

    fn draw_error(mut self, error: eyre::Report) {
        let big_font = Font::new(&self.typeface, 36.0);
        let small_font: skia_safe::Handle<_> = Font::new(&self.typeface, 12.0);

        self.canvas
            .draw_str("ERROR", (100, 200), &big_font, &self.black_paint);
        self.y = 250.0;

        for e in error.chain() {
            self.canvas.draw_str(
                format!("{e}"),
                (100.0, self.y),
                &small_font,
                &self.black_paint,
            );
            self.y += 20.0;
        }
    }
}

pub fn stops_png(
    render_target: RenderTarget,
    layout: Layout,
    config_file: &ConfigFile,
) -> Result<Vec<u8>> {
    let image_data = render_ctx(render_target, config_file, |canvas| {
        let ctx = Render::new(canvas, config_file)?;
        ctx.draw(&layout)?;

        Ok(())
    })?;

    Ok(image_data)
}

pub fn error_png(
    render_target: RenderTarget,
    config_file: &ConfigFile,
    error: eyre::Report,
) -> Result<Vec<u8>> {
    let data = render_ctx(render_target, config_file, move |canvas| {
        let ctx = Render::new(canvas, config_file)?;
        ctx.draw_error(error);
        Ok(())
    })?;

    Ok(data)
}
