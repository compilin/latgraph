use crate::{
    app::LatGraphSettings,
    ringbuf::{Ping, RingBuffer},
};
use conrod_core::Borderable;
use std::time::{Duration, Instant};

use conrod_core::{
    builder_method,
    color::{self, Color},
    position::{range::Range, Padding},
    widget, widget_ids, Colorable, Positionable, Rect, Sizeable, Widget, WidgetCommon, WidgetStyle,
};
use log::*;

#[derive(Debug, WidgetCommon)]
pub struct LatencyGraphWidget<'a> {
    #[conrod(common_builder)]
    common: widget::CommonBuilder,
    buffer: &'a RingBuffer,
    settings: &'a LatGraphSettings,
    style: Style,
    is_mouse_over_window: bool,
}

widget_ids!(
    struct Ids {
        border,
        hover_highlight,
        x_ticks[],
        x_tick_label,
        y_ticks[],
        y_tick_label,
        y_min_tick,
        y_min_label,
        y_max_tick,
        y_max_label,
        y_avg_tick,
        y_avg_label,
        y_minmax_bar,
        bars[],
    }
);

const ZOOM_BASE: f64 = 1.2;
pub const ZOOM_DEFAULT: u16 = 8;
const ZOOM_MAX: f64 = 20.;
// Min,max distance between horizontal ticks, in pixels
const TICK_MIN_STEP: u128 = 75;
const TICK_MAX_STEP: u128 = 200;
const TICK_STEPS: [u128; 12] = [
    // Allowed values for the distance in milliseconds between ticks
    100, 250, 500, 1000, 2500, 5000, 10_000, 20_000, 30_000, 60_000, 120_000, 240_000,
];

const GRAPH_AREA_PADDING: Padding = Padding {
    x: Range {
        start: 10., // left
        end: 50.,   // right
    },
    y: Range {
        start: 25., // bottom
        end: 10.,   // top
    },
};

pub struct State {
    ids: Ids,
    tick_step: usize, // Index of the current tick step in teh TICK_STEPS array
}

type Zoom = (u16, u16);

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
    pub fn new(
        buffer: &'a RingBuffer,
        settings: &'a LatGraphSettings,
        is_mouse_over_window: bool,
    ) -> Self {
        Self {
            common: widget::CommonBuilder::default(),
            buffer,
            settings,
            style: Style::default(),
            is_mouse_over_window,
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
            tick_step: 0,
        }
    }
    fn style(&self) -> <Self as Widget>::Style {
        self.style
    }

    fn update(self, args: widget::UpdateArgs<'_, '_, '_, '_, Self>) -> Zoom {
        let widget::UpdateArgs {
            id,
            rect: widget_area,
            state,
            ui,
            ..
        } = args;

        let graph_area = widget_area.padding(GRAPH_AREA_PADDING);
        let x_axis_area = Rect::from_corners(graph_area.bottom_right(), widget_area.bottom_left());
        let y_axis_area = Rect::from_corners(graph_area.bottom_right(), widget_area.top_right());

        let border_color = self.style.border_color(ui.theme());
        let inputs = ui.widget_input(id);
        let mut is_over_x = false;
        let mut is_over_y = false;
        if let Some(mouse) = inputs.mouse() {
            let highlight_rect = if self.is_mouse_over_window && x_axis_area.is_over(mouse.rel_xy())
            {
                is_over_x = true;
                Some(x_axis_area)
            } else if self.is_mouse_over_window && y_axis_area.is_over(mouse.rel_xy()) {
                is_over_y = true;
                Some(y_axis_area)
            } else {
                None
            };
            if let Some(rect) = highlight_rect {
                let minmax_bar_color = border_color.alpha(0.15);

                widget::Rectangle::fill(rect.dim())
                    .xy(rect.xy())
                    .color(minmax_bar_color)
                    .parent(id)
                    .graphics_for(id)
                    .set(state.ids.hover_highlight, ui);
            }
        }

        let inputs = ui.widget_input(id);
        let mut zoom = self.settings.zoom;
        let delta_zoom = inputs
            .scrolls()
            .map(|scroll| {
                if scroll.y != 0. {
                    -f64::signum(scroll.y)
                } else {
                    0.
                }
            })
            .fold(0., |acc, val| acc + val);
        if delta_zoom != 0. {
            if is_over_x {
                let old_zoom = zoom.0;
                zoom.0 = (zoom.0 as f64 + delta_zoom).clamp(0., ZOOM_MAX) as u16;
                debug!("Adjusting horizontal zoom {} -> {}", old_zoom, zoom.0);
            } else if is_over_y {
                let old_zoom = zoom.1;
                zoom.1 = (zoom.1 as f64 + delta_zoom).clamp(0., ZOOM_MAX) as u16;
                debug!("Adjusting vertical zoom {} -> {}", old_zoom, zoom.1);
            }
        }

        /* PING BARS */
        let bar_color = self.style.color(ui.theme()).alpha(0.5);
        let missing_color = color::rgba_bytes(192, 64, 32, 0.3);
        let bar_width = f64::powi(ZOOM_BASE, zoom.0 as i32);
        let now = Instant::now();
        let x_step = bar_width + 1.;
        let x_offset = if self.buffer.len() > 0 && self.settings.running {
            // Offset as a function of time since the last packet was sent
            now.saturating_duration_since(self.buffer[self.buffer.get_end_index()].sent_time())
                .as_micros() as f64
                / self.settings.delay.as_micros() as f64
        } else {
            1.
        };
        let x_offset = bar_width * x_offset.clamp(0., 1.);
        let nb_points = usize::min(self.buffer.len(), (graph_area.w() / x_step) as usize + 2);
        let mut min_lat = u128::MAX;
        let mut max_lat = 0;
        let mut avg_lat = 0;
        let mut nb_lat = 0;

        let lat_to_y = |lat| graph_area.bottom() + f64::sqrt(lat as f64) * f64::powi(ZOOM_BASE, zoom.1 as i32) * 2.;

        if state.ids.bars.len() < nb_points {
            state.update(|state| {
                state
                    .ids
                    .bars
                    .resize(nb_points, &mut ui.widget_id_generator())
            });
        }
        for (i, ping) in self.buffer.iter_rev().take(nb_points).enumerate() {
            let x = graph_area.right() - (i as f64 * x_step + x_offset);

            match ping {
                Ping::Received(_, lat) => {
                    let y = lat_to_y(lat);
                    if let Some(rct) =
                        Rect::from_corners([x, graph_area.bottom()], [x + bar_width, y])
                            .overlap(graph_area)
                    {
                        widget::Rectangle::fill(rct.dim())
                            .xy(rct.xy())
                            .color(bar_color)
                            .parent(id)
                            .graphics_for(id)
                            .set(state.ids.bars[i], ui);
                    }
                    if lat < min_lat {
                        min_lat = lat;
                    }
                    if lat > max_lat {
                        max_lat = lat;
                    }
                    avg_lat += lat;
                    nb_lat += 1;
                }
                Ping::Sent(time) => {
                    if let Some(rct) = Rect::from_corners(
                        [x, graph_area.bottom()],
                        [x + bar_width, graph_area.top()],
                    )
                    .overlap(graph_area)
                    {
                        let age = now.saturating_duration_since(time);
                        let alpha = (age.as_millis() as f32 - 1000.) / 1000.;
                        let alpha = alpha.clamp(0., 1.);
                        widget::Rectangle::fill(rct.dim())
                            .xy(rct.xy())
                            .color(missing_color.clone().alpha(alpha))
                            .parent(id)
                            .graphics_for(id)
                            .set(state.ids.bars[i], ui);
                    }
                }
            };
            if x < graph_area.left() {
                // Add the first point that is outside the rectangle to complete the line, then break
                break;
            }
        }

        /* WIDGET BORDER */
        widget::Rectangle::outline_styled(
            graph_area.dim(),
            widget::line::Style::solid().thickness(self.style.border(ui.theme())),
        )
        .xy(graph_area.xy())
        .color(border_color)
        .parent(id)
        .graphics_for(id)
        .set(state.ids.border, ui);

        /* X TICKS */
        let tick_step = update_ticks_step(state.tick_step, x_step, self.settings.delay);
        if tick_step != state.tick_step {
            state.update(|state| state.tick_step = tick_step);
        }

        let tick_dist =
            TICK_STEPS[tick_step] as f64 * x_step / self.settings.delay.as_millis() as f64;
        let x_tick_nb = (graph_area.w() / tick_dist).ceil() as usize;
        if x_tick_nb > state.ids.x_ticks.len() {
            state.update(|state| {
                state
                    .ids
                    .x_ticks
                    .resize(x_tick_nb, &mut ui.widget_id_generator());
            });
        }
        for i in 0..x_tick_nb {
            let x = graph_area.right() - i as f64 * tick_dist;

            widget::Line::abs([x, graph_area.bottom()], [x, graph_area.bottom() - 10.])
                .color(border_color)
                .parent(id)
                .graphics_for(id)
                .set(state.ids.x_ticks[i], ui);

            if i == x_tick_nb - 1 {
                let dur = Duration::from_millis(TICK_STEPS[tick_step] as u64 * i as u64);
                widget::Text::new(&format!("{:?}", dur))
                    .xy([x, graph_area.bottom() - 20.])
                    .wh([20., 20.])
                    .center_justify()
                    .font_size(8)
                    .color(border_color)
                    .parent(id)
                    .graphics_for(id)
                    .set(state.ids.x_tick_label, ui);
            }
        }

        /* Y TICKS */
        if nb_lat > 0 {
            const TICK_LENGTH: f64 = 10.;

            let mut set_tick = |lat: u128, rect: Rect, y: f64, tick_id, label_id| {
                widget::Line::abs(
                    [graph_area.right(), y],
                    [graph_area.right() + TICK_LENGTH, y],
                )
                .color(border_color)
                .parent(id)
                .graphics_for(id)
                .set(tick_id, ui);

                widget::Text::new(&format_latency(lat))
                    .xy(rect.xy())
                    .wh(rect.dim())
                    .left_justify()
                    .font_size(8)
                    .color(border_color)
                    .parent(id)
                    .graphics_for(id)
                    .set(label_id, ui);
            };

            let avg_lat = avg_lat / nb_lat;
            let avg_y = lat_to_y(avg_lat);
            let avg_rect =
                Rect::from_xy_dim([graph_area.right() + TICK_LENGTH + 22., avg_y], [40., 10.]);
            if avg_y < graph_area.top() {
                set_tick(
                    avg_lat,
                    avg_rect,
                    avg_y,
                    state.ids.y_avg_tick,
                    state.ids.y_avg_label,
                );
            }

            let min_y = lat_to_y(min_lat);
            let min_rect = Rect::from_xy_dim(
                [avg_rect.x(), f64::min(avg_y - avg_rect.h(), min_y)],
                avg_rect.dim(),
            );
            set_tick(
                min_lat,
                min_rect,
                min_y,
                state.ids.y_min_tick,
                state.ids.y_min_label,
            );

            let max_y = lat_to_y(max_lat);
            if max_y <= graph_area.top() {
                let max_rect = Rect::from_xy_dim(
                    [avg_rect.x(), f64::max(avg_y + avg_rect.h(), max_y)],
                    avg_rect.dim(),
                );

                set_tick(
                    max_lat,
                    max_rect,
                    max_y,
                    state.ids.y_max_tick,
                    state.ids.y_max_label,
                );
            }

            let minmax_bar_color = border_color.alpha(0.15);
            let minmax_rect = Rect::from_corners(
                [graph_area.right(), min_y],
                [
                    graph_area.right() + TICK_LENGTH,
                    f64::min(max_y, graph_area.top()),
                ],
            );

            widget::Rectangle::fill(minmax_rect.dim())
                .xy(minmax_rect.xy())
                .color(minmax_bar_color)
                .parent(id)
                .graphics_for(id)
                .set(state.ids.y_minmax_bar, ui);
        }
        trace!(
            "Updating ringbuf over area {:?} widget with {} points, zoom: {:?}",
            graph_area,
            nb_points,
            zoom,
        );

        zoom
    }
}

fn update_ticks_step(old_step: usize, step_width: f64, delay: Duration) -> usize {
    let delay = delay.as_millis();
    let step_width = step_width as u128;
    let mut step = old_step;
    // Find the closest tick step that results in a distance within the given range
    while TICK_STEPS[step] * step_width / delay > TICK_MAX_STEP {
        if step > 0 {
            step -= 1;
        } else {
            break;
        }
    }
    while TICK_STEPS[step] * step_width / delay < TICK_MIN_STEP {
        if step < TICK_STEPS.len() - 1 {
            step += 1;
        } else {
            break;
        }
    }
    if old_step != step {
        debug!(
            "Updating tick_step: {:?} ({}) => {:?} ({}), pixel dist: {}",
            Duration::from_millis(TICK_STEPS[old_step] as u64),
            old_step,
            Duration::from_millis(TICK_STEPS[step] as u64),
            step,
            TICK_STEPS[step] * step_width / delay
        );
    }
    step
}

fn format_latency(lat: u128) -> String {
    if lat < 1000 {
        lat.to_string() + "ms"
    } else if lat < 60000 {
        format!("{:.2}s", lat as f32 / 1000.)
    } else {
        String::from(">1m")
    }
}

impl Colorable for LatencyGraphWidget<'_> {
    builder_method!(color { style.color = Some(Color) });
}

impl Borderable for LatencyGraphWidget<'_> {
    builder_method!(border { style.border = Some(f64) });
    builder_method!(border_color { style.border_color = Some(Color) });
}
