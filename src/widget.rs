use crate::ringbuf::{Ping, RingBuffer};
use std::time::{Duration, Instant};

use conrod_core::{
    builder_method,
    color::{rgba_bytes, Color, RED},
    widget, widget_ids, Colorable, Point, Positionable, Rect, Sizeable, Widget, WidgetCommon,
    WidgetStyle,
};
use log::*;

#[derive(Debug, WidgetCommon)]
pub struct LatencyGraphWidget<'a> {
    #[conrod(common_builder)]
    common: widget::CommonBuilder,
    buffer: &'a RingBuffer,
    delay: Duration,
    zoom: u16,
    style: Style,
}

widget_ids!(
    struct Ids {
        border,
        ticks[],
        paths[],
        rects[],
        points[],
    }
);

const ZOOM_BASE: f64 = 1.2;

pub struct State {
    ids: Ids,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, WidgetStyle)]
pub struct Style {
    #[conrod(default = "theme.shape_color")]
    pub color: Option<Color>,

    #[conrod(default = "1.0")]
    pub line_thickness: Option<f64>,

    #[conrod(default = "2.0")]
    pub point_thickness: Option<f64>,
}

impl<'a> LatencyGraphWidget<'a> {
    pub fn new(buffer: &'a RingBuffer, delay: Duration, zoom: u16) -> Self {
        Self {
            common: widget::CommonBuilder::default(),
            buffer,
            delay,
            zoom,
            style: Style::default(),
        }
    }

    pub fn line_thickness(mut self, thickness: f64) -> Self {
        self.style.line_thickness = Some(thickness);
        self
    }

    pub fn point_thickness(mut self, thickness: f64) -> Self {
        self.style.point_thickness = Some(thickness);
        self
    }
}

impl Widget for LatencyGraphWidget<'_> {
    type State = State;
    type Style = Style;
    type Event = ();

    fn init_state(&self, id_gen: widget::id::Generator<'_>) -> <Self as Widget>::State {
        State {
            ids: Ids::new(id_gen),
        }
    }
    fn style(&self) -> <Self as Widget>::Style {
        self.style
    }

    fn update(self, args: widget::UpdateArgs<'_, '_, '_, '_, Self>) {
        let widget::UpdateArgs {
            id,
            rect,
            state,
            ui,
            ..
        } = args;

        enum Current {
            // What we're currently drawing
            Segment(Vec<Point>),
            Missing(f64, f64),
        }
        use Current::*;

        let x_step = f64::powf(ZOOM_BASE, self.zoom as f64);
        let x_offset = if self.buffer.len() > 0 {
            x_step
                * self.buffer[self.buffer.len() - 1]
                    .sent_time()
                    .elapsed()
                    .as_micros() as f64
                / self.delay.as_micros() as f64
        } else {
            0.
        };

        let mut current = Segment(Vec::new());
        let mut segments = Vec::new();
        let mut points = Vec::with_capacity(usize::min(self.buffer.len(), (rect.w() / x_step) as usize + 1));
        let mut missing_rects = Vec::new();
        for (i, ping) in self.buffer.iter_rev().enumerate() {
            let x = rect.right() - (i as f64 * x_step + x_offset);

            current = match ping {
                Ping::Received(_, lat) => {
                    let y = rect.bottom() + lat as f64;
                    points.push([x, y]);
                    match &mut current {
                        Segment(pts) => {
                            pts.push([x, y]);
                            current
                        }
                        Missing(from, to) => {
                            if self.delay * (i as u32) > Duration::from_secs(1) { // Only consider packets lost after 1s
                                missing_rects.push([*from, *to]);
                            }
                            Segment(vec![[x, y]])
                        }
                    }
                }
                Ping::Sent(_) => match current {
                    Segment(pts) => {
                        if pts.len() > 1 {
                            segments.push(pts);
                        }
                        Missing(x, x)
                    }
                    Missing(_, to) => Missing(x, to),
                },
            };
            if x < rect.left() {
                // Add the first point that is outside the rectangle to complete the line, then break
                break;
            }
        }
        match current {
            Segment(pts) => {
                if !pts.is_empty() {
                    segments.push(pts);
                }
            }
            Missing(from, to) => {
                missing_rects.push([from, to]);
            }
        }

        trace!(
            "Updating ringbuf over area {:?} widget with {} points in {} segments with {} missing rects. First rect: {:?}",
            rect,
            points.len(),
            segments.len(),
            missing_rects.len(),
            missing_rects.first(),
        );
        {
            let mut id_gen = ui.widget_id_generator();
            // Make sure each list of ids we have has enough for the corresponding list of widgets
            macro_rules! gen_ids {
                ($list:ident, $id_list:ident) => {
                    if $list.len() > state.ids.$id_list.len() {
                        state.update(|state| state.ids.$id_list.resize($list.len(), &mut id_gen));
                    }
                };
            }
            gen_ids!(segments, paths);
            gen_ids!(missing_rects, rects);
            gen_ids!(points, points);
        }

        /* WIDGET BORDER */
        let color = self.style.color(ui.theme());
        widget::Rectangle::outline(rect.dim())
            .xy(rect.xy())
            .color(color)
            .parent(id)
            .graphics_for(id)
            .set(state.ids.border, ui);

        /* SEGMENTS */
        let color = color.alpha(0.5);
        let thickness = self.style.line_thickness(ui.theme());
        for (i, segment) in segments.into_iter().enumerate() {
            widget::PointPath::new(segment)
                .wh(rect.dim())
                .xy(rect.xy())
                .color(color)
                .thickness(thickness)
                .parent(id)
                .graphics_for(id)
                .set(state.ids.paths[i], ui);
        }

        /* LOST DATAGRAM BACKGROUND */
        let color = rgba_bytes(192, 64, 32, 0.3);
        for (i, [from, to]) in missing_rects.into_iter().enumerate() {
            let rect = Rect::from_corners(
                [from - x_step / 2., rect.bottom()],
                [to + x_step / 2., rect.top()],
            );

            widget::Rectangle::fill(rect.dim())
                .xy(rect.xy())
                .color(color)
                .parent(id)
                .graphics_for(id)
                .set(state.ids.rects[i], ui);
        }

        /* PING POINTS */
        let color = self.style.color(ui.theme());
        let radius = self.style.point_thickness(ui.theme()) / 2.;
        for (i, point) in points.iter().enumerate() {
            widget::Circle::fill(radius)
                .xy(*point)
                .color(color)
                .parent(id)
                .graphics_for(id)
                .set(state.ids.points[i], ui);
        }
    }
}

impl Colorable for LatencyGraphWidget<'_> {
    builder_method!(color { style.color = Some(Color) });
}
