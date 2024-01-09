use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::Hasher,
    sync::Arc,
};

use crate::{
    config::ConfigFile,
    layout::{Agency, Layout, Line, Row},
};
use chrono::{prelude::*, Duration};
use eyre::{bail, eyre, Result};
use skia_safe::{
    gradient_shader::GradientShaderColors, utils::text_utils::Align, Bitmap, Canvas, Color,
    Color4f, Font, FontMgr, ImageInfo, Paint, Point, Rect, Shader, TextBlob, TileMode, Typeface,
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

pub struct SharedRenderData {
    black_paint: Paint,
    black_paint_heavy: Paint,
    grey_paint: Paint,
    light_grey_paint: Paint,
    white_paint: Paint,
    typeface: Typeface,
    font: Font,
}

struct Render<'a> {
    shared: Arc<SharedRenderData>,

    line_id_bubble_paint: Paint,

    canvas: &'a mut Canvas,

    width: f32,
    height: f32,
    y: f32,

    x_midpoint: f32,
}

impl SharedRenderData {
    pub fn new() -> Arc<Self> {
        let mut black_paint_heavy = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
        black_paint_heavy.set_stroke_width(2.0);

        let font_mgr = FontMgr::new();
        let typeface = font_mgr
            .new_from_data(include_bytes!("../media/OpenSansEmoji.ttf"), None)
            .unwrap();

        Arc::new(Self {
            black_paint: Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None),
            black_paint_heavy,

            grey_paint: Paint::new(Color4f::new(0.7, 0.7, 0.7, 1.0), None),
            light_grey_paint: Paint::new(Color4f::new(0.8, 0.8, 0.8, 1.0), None),
            white_paint: Paint::new(Color4f::new(1.0, 1.0, 1.0, 1.0), None),

            font: Font::new(&typeface, 24.0),
            typeface,
        })
    }
}

impl<'a> Render<'a> {
    fn new(
        canvas: &'a mut Canvas,
        shared: Arc<SharedRenderData>,
        config: &ConfigFile,
    ) -> Result<Self> {
        let mut line_bubble_paint = Paint::new(Color4f::new(0.8, 0.8, 0.8, 1.0), None);
        line_bubble_paint.set_anti_alias(true);

        Ok(Self {
            canvas,
            shared,

            line_id_bubble_paint: line_bubble_paint,

            width: config.layout.width as f32,
            height: config.layout.height as f32,
            y: 0.0,

            x_midpoint: config.layout.width as f32 / 2.0,
        })
    }

    fn draw_row(&mut self, row: &Row, x1: f32, x2: f32) -> Result<()> {
        if self.y > 0.0 {
            self.canvas
                .draw_line((x1, self.y), (x2, self.y), &self.shared.black_paint_heavy);
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

            let line_id_bounds = self.draw_line_id_bubble(&line.id, x)?;

            self.canvas.draw_str(
                &line.destination,
                (x + line_id_bounds.width(), self.y),
                &self.shared.font,
                &self.shared.black_paint,
            );

            self.draw_departure_times(x2, line);

            if idx < (lines_len - 1) {
                self.canvas.draw_line(
                    (x1 + 40.0, self.y + 15.0),
                    (x2 - 40.0, self.y + 15.0),
                    &self.shared.grey_paint,
                );
                self.y += 48.0;
            } else {
                self.y += 15.0;
            }
        }

        Ok(())
    }

    fn draw_departure_times(&mut self, x: f32, line: &Line) {
        let mins = line.departure_minutes_str();
        let time_text = format!("{mins} min");

        let time_point = (x - 20.0, self.y);

        let time_rect_exact = self.text_bounds_right_align(&time_text, time_point);
        let time_rect = time_rect_exact.with_outset((15.0, 10.0));

        let time_rect_left = Rect::new(
            time_rect.left - 25.0,
            time_rect_exact.top,
            time_rect.left,
            time_rect.bottom,
        );

        let white_opaque = Color::from_argb(255, 255, 255, 255);
        let white_transparent = Color::from_argb(0, 255, 255, 255);

        let mut gradiant = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
        gradiant.set_shader(Shader::linear_gradient(
            (
                (
                    time_rect_left.right,
                    time_rect_left.top + (0.5 * time_rect_left.height()),
                ),
                (
                    time_rect_left.left,
                    time_rect_left.top + (0.5 * time_rect_left.height()),
                ),
            ),
            GradientShaderColors::Colors(&[white_opaque, white_transparent]),
            Some(&[0.0f32, 1.0] as &[f32]),
            TileMode::Repeat,
            None,
            None,
        ));

        self.canvas.draw_rect(time_rect, &self.shared.white_paint);

        self.canvas.draw_rect(time_rect_left, &gradiant);

        self.canvas.draw_str_align(
            time_text,
            time_point,
            &self.shared.font,
            &self.shared.black_paint,
            Align::Right,
        );
    }

    fn map_range(from_range: (f32, f32), to_range: (f32, f32), s: f32) -> f32 {
        to_range.0 + (s - from_range.0) * (to_range.1 - to_range.0) / (from_range.1 - from_range.0)
    }

    fn text_bounds(&mut self, text: &str, (x, y): (f32, f32)) -> Rect {
        let (text_width, text_measurements) = self
            .shared
            .font
            .measure_str(text, Some(&self.shared.black_paint));
        Rect::new(x, y + text_measurements.top, x + text_width, y)
    }

    fn text_bounds_right_align(&mut self, text: &str, (x, y): (f32, f32)) -> Rect {
        let (text_width, text_measurements) = self
            .shared
            .font
            .measure_str(text, Some(&self.shared.black_paint));
        Rect::new(x - text_width, y + text_measurements.top, x, y)
    }

    fn draw_line_id_bubble(&mut self, line_id: &str, x: f32) -> Result<Rect> {
        let blob = TextBlob::new(line_id, &self.shared.font)
            .ok_or(eyre!("failed to construct skia text blob"))?;

        let bounds = self
            .text_bounds(line_id, (x, self.y))
            .with_outset((10.0, 10.0));

        let mut color_hasher = DefaultHasher::new();
        color_hasher.write(line_id.as_bytes());
        let color_hash = color_hasher.finish() as f32;

        // map a value in the space 0..u64::MAX to the space 0.3..0.9
        let color = Self::map_range((0.0, u64::MAX as f32), (0.5, 0.9), color_hash);

        self.line_id_bubble_paint
            .set_color4f(Color4f::new(color, color, color, 1.0), None);

        self.canvas
            .draw_round_rect(bounds, 24.0, 24.0, &self.line_id_bubble_paint);

        self.canvas
            .draw_text_blob(&blob, (x, self.y), &self.shared.black_paint);

        Ok(bounds)
    }

    fn draw_footer(&mut self, all_agencies: &HashMap<String, DateTime<Utc>>) {
        let bottom_box_y = self.height - 40.0;

        self.canvas.draw_rect(
            Rect::new(0.0, bottom_box_y, self.width, self.height),
            &self.shared.light_grey_paint,
        );

        self.canvas.draw_line(
            (0.0, bottom_box_y),
            (self.width, bottom_box_y),
            &self.shared.black_paint_heavy,
        );

        let now = Local::now();
        let time = now.format("%a %b %d - %H:%M").to_string();

        let mut agency_str = String::new();

        for (agency_name, live_time) in all_agencies {
            let age = now.signed_duration_since(*live_time);

            let agency = crate::agencies::agency_readable(agency_name);

            let status = if age < Duration::minutes(5) {
                // Checkbox emoji
                String::from("\u{2611}")
            } else {
                format!("{} mins", age.num_minutes())
            };

            agency_str.push_str(&format!(" {agency}: {status},"));
        }
        agency_str.pop();

        self.canvas.draw_str_align(
            agency_str,
            (self.width - 20.0, self.height - 10.0),
            &self.shared.font,
            &self.shared.black_paint,
            Align::Right,
        );

        self.canvas.draw_str_align(
            time,
            (20.0, self.height - 10.0),
            &self.shared.font,
            &self.shared.black_paint,
            Align::Left,
        );
    }

    fn draw_text_row(&mut self, text: &str, x1: f32, x2: f32) {
        self.canvas.draw_rect(
            Rect::new(x1, self.y, x2, self.y + 40.0),
            &self.shared.light_grey_paint,
        );
        self.y += 28.0;

        self.canvas.draw_str_align(
            text,
            ((x1 + x2) / 2.0, self.y),
            &self.shared.font,
            &self.shared.black_paint,
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
            &self.shared.black_paint_heavy,
        );

        self.draw_footer(&layout.all_agencies);

        Ok(())
    }

    fn draw_error(mut self, error: eyre::Report) {
        let big_font = Font::new(&self.shared.typeface, 36.0);
        let small_font: skia_safe::Handle<_> = Font::new(&self.shared.typeface, 12.0);

        self.canvas
            .draw_str("ERROR", (100, 200), &big_font, &self.shared.black_paint);
        self.y = 250.0;

        for e in error.chain() {
            self.canvas.draw_str(
                format!("{e}"),
                (100.0, self.y),
                &small_font,
                &self.shared.black_paint,
            );
            self.y += 20.0;
        }
    }
}

pub fn stops_png(
    render_target: RenderTarget,
    shared: Arc<SharedRenderData>,
    layout: Layout,
    config_file: &ConfigFile,
) -> Result<Vec<u8>> {
    let image_data = render_ctx(render_target, config_file, |canvas| {
        let ctx = Render::new(canvas, shared, config_file)?;
        ctx.draw(&layout)?;

        Ok(())
    })?;

    Ok(image_data)
}

pub fn error_png(
    render_target: RenderTarget,
    shared: Arc<SharedRenderData>,
    config_file: &ConfigFile,
    error: eyre::Report,
) -> Result<Vec<u8>> {
    let data = render_ctx(render_target, config_file, move |canvas| {
        let ctx = Render::new(canvas, shared, config_file)?;
        ctx.draw_error(error);
        Ok(())
    })?;

    Ok(data)
}
