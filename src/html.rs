use eyre::Result;

use crate::layout::Layout;
use askama::Template;

#[derive(Template)]
#[template(path = "stops.html")]

struct StopsTemplate {
    layout: Layout,
}

pub fn stops_html(layout: Layout) -> Result<String> {
    let html = StopsTemplate { layout }.render()?;

    Ok(html)
}
