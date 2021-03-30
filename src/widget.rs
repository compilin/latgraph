use crate::ringbuf::{Ping, RingBuffer};
use conrod_core::Borderable;
use std::time::Duration;

use conrod_core::{
    builder_method,
    color::{rgba_bytes, Color},
    widget, widget_ids, Colorable, Positionable, Rect, Widget, WidgetCommon, WidgetStyle,
};
use log::*;

#[derive(Debug, WidgetCommon)]
pub struct LatencyGraphWidget<'a> {
    #[conrod(common_builder)]
    common: widget::CommonBuilder,
    buffer: &'a RingBuffer,
    delay: Duration,
    zoom: Zoom,
    style: Style,
}

widget_ids!(
    struct Ids {
        border,
        ticks[],
        bars[],
    }
);

const ZOOM_BASE: f64 = 1.2;
const ZOOM_DEFAULT: u16 = 8;
const ZOOM_MAX: f64 = 20.;

pub struct State {
    ids: Ids,
}

#[derive(Copy, Clone, Debug)]
pub struct Zoom {
    vertical: u16,
    horizontal: u16,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, WidgetStyle)]
pub struct Style {
    #[conrod(default = "theme.border_color")]
    pub color: Option<Color>,
    #[conrod(default = "theme.shape_color")]
    pub missing_color: Option<Color>,

    #[conrod(default = "1.0")]
    pub border: Option<f64>,
    #[conrod(default = "theme.border_color")]
    pub border_color: Option<Color>,
}

impl<'a> LatencyGraphWidget<'a> {
    pub fn new(buffer: &'a RingBuffer, delay: Duration, zoom: Zoom) -> Self {
        Self {
            common: widget::CommonBuilder::default(),
            buffer,
            delay,
            zoom,
            style: Style::default(),
        }
    }

    builder_method!(pub missing_color { style.missing_color = Some(Color) });
}

impl Widget for LatencyGraphWidget<'_> {
    type State = State;
    type Style = Style;
    type Event = Zoom;

    fn init_state(&self, id_gen: widget::id::Generator<'_>) -> <Self as Widget>::State {
        State {
            ids: Ids::new(id_gen),
        }
    }
    fn style(&self) -> <Self as Widget>::Style {
        self.style
    }

    fn update(self, args: widget::UpdateArgs<'_, '_, '_, '_, Self>) -> Zoom {
        let widget::UpdateArgs {
            id,
            rect,
            state,
            ui,
            ..
        } = args;

        let mut zoom = self.zoom;
        {
            let mut horizontal = self.zoom.horizontal as f64;
            for scroll in ui.widget_input(id).scrolls() {
                if scroll.y != 0. {
                    horizontal += f64::signum(scroll.y);
                }
            }
            zoom.horizontal = horizontal.clamp(0., ZOOM_MAX) as u16;
        }

        /* WIDGET BORDER */
        widget::Rectangle::outline_styled(
            rect.dim(),
            widget::line::Style::solid().thickness(self.style.border(ui.theme())),
        )
        .xy(rect.xy())
        .color(self.style.border_color(ui.theme()))
        .parent(id)
        .graphics_for(id)
        .set(state.ids.border, ui);

        /* PING BARS */
        let bar_color = self.style.color(ui.theme()).alpha(0.5);
        let missing_color = rgba_bytes(192, 64, 32, 0.3);

        let bar_width = f64::powf(ZOOM_BASE, zoom.horizontal as f64);
        let x_step = bar_width + 1.;
        let x_offset = if self.buffer.len() > 0 {
            bar_width
                * self.buffer[self.buffer.len() - 1]
                    .sent_time()
                    .elapsed()
                    .as_micros() as f64
                / self.delay.as_micros() as f64
        } else {
            0.
        };
        let nb_points = usize::min(self.buffer.len(), (rect.w() / x_step) as usize + 1);
        if state.ids.bars.len() < nb_points {
            let mut id_gen = ui.widget_id_generator();
            state.update(|state| state.ids.bars.resize(nb_points, &mut id_gen));
        }
        for (i, ping) in self.buffer.iter_rev().take(nb_points).enumerate() {
            let x = rect.right() - (i as f64 * x_step + x_offset);

            match ping {
                Ping::Received(_, lat) => {
                    let y = rect.bottom() + lat as f64;
                    let rct = Rect::from_corners(
                        [x, rect.bottom()],
                        [x + bar_width, y],
                    );
                    widget::Rectangle::fill(rct.dim())
                        .xy(rct.xy())
                        .color(bar_color)
                        .parent(id)
                        .graphics_for(id)
                        .set(state.ids.bars[i], ui);
                }
                Ping::Sent(_) => {
                    let rct = Rect::from_corners(
                        [x, rect.bottom()],
                        [x + bar_width, rect.top()],
                    );
                    if self.delay * (i as u32) > Duration::from_secs(1) {
                        // Only consider packets lost after 1s
                        widget::Rectangle::fill(rct.dim())
                            .xy(rct.xy())
                            .color(missing_color)
                            .parent(id)
                            .graphics_for(id)
                            .set(state.ids.bars[i], ui);
                    }
                }
            };
            if x < rect.left() {
                // Add the first point that is outside the rectangle to complete the line, then break
                break;
            }
        }

        trace!(
            "Updating ringbuf over area {:?} widget with {} points, zoom: {:?}",
            rect,
            nb_points,
            zoom,
        );

        zoom
    }
}

impl Colorable for LatencyGraphWidget<'_> {
    builder_method!(color { style.color = Some(Color) });
}

impl Borderable for LatencyGraphWidget<'_> {
    builder_method!(border { style.border = Some(f64) });
    builder_method!(border_color { style.border_color = Some(Color) });
}

impl Default for Zoom {
    fn default() -> Self {
        Zoom {
            horizontal: ZOOM_DEFAULT,
            vertical: ZOOM_DEFAULT,
        }
    }
}
