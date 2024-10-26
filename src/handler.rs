use std::sync::Arc;

use axum::async_trait;
use eyre::{Context, Result};
use kindling::Orientation;

use crate::{
    api_client::DataAccess,
    layout::{data_to_layout, Layout},
    render::{Render, SharedRenderData},
    ConfigFile,
};

pub(crate) struct TransitHandler {
    pub(crate) data_access: Arc<DataAccess>,
    pub(crate) config_file: ConfigFile,
    pub(crate) shared: Arc<SharedRenderData>,
}

#[async_trait]
impl kindling::Handler for TransitHandler {
    type Data = Layout;

    async fn load(&self) -> Result<Self::Data> {
        let stop_data = self
            .data_access
            .load_stop_data(self.config_file.clone())
            .await
            .wrap_err("load stop data")?;

        let layout = data_to_layout(stop_data, &self.config_file);

        Ok(layout)
    }

    fn draw(&self, canvas: &skia_safe::Canvas, layout: Layout) -> Result<()> {
        let ctx = Render::new(canvas, self.shared.clone())?;
        ctx.draw(&layout)?;

        Ok(())
    }

    fn orientation() -> Orientation {
        Orientation::Landscape
    }
}
