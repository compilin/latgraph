use crate::ringbuf::{Ping, RingBuffer};
use std::time::Instant;

use conrod_core::{
    builder_method,
    color::{Color, RED},
    widget, widget_ids, Colorable, Widget, WidgetCommon, WidgetStyle,
};
use log::*;

#[derive(Debug, WidgetCommon)]
pub struct LatencyGraphWidget<'a> {
    #[conrod(common_builder)]
    common: widget::CommonBuilder,
    buffer: &'a mut RingBuffer,
    style: Style,
}

widget_ids!(
    struct Ids {
        paths[],
        rects[],
        points[]
    }
);

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
    pub fn new(buffer: &'a mut RingBuffer) -> Self {
        Self {
            common: widget::CommonBuilder::default(),
            buffer,
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
        use conrod_core::{Positionable, Sizeable};
        let widget::UpdateArgs {
            id,
            rect,
            state,
            ui,
            ..
        } = args;
        let now = Instant::now();

        let mut first_point = None;
        let mut last_point = None;
        let mut points = Vec::new();
        let mut segments = Vec::new();
        let mut segment = Vec::new();
        for (i, ping) in self.buffer.iter_rev().with_index() {
            let (time, lat) = match ping {
                Ping::Sent(time) => (time, None),
                Ping::Received(time, lat) => (time, Some(lat)),
            };
            let x = rect.right() - now.saturating_duration_since(time).as_millis() as f64 / 100.0; // TODO implement variable zoom
            let break_after = x < rect.left();
            // let x = utils::clamp(x, rect.left(), rect.right());

            match lat {
                Some(lat) => {
                    let y = rect.bottom() + lat as f64;
                    // let y = utils::clamp(rect.bottom() + lat as f64, rect.bottom(), rect.top());
                    points.push((i, [x, y]));
                    segment.push((i, [x, y]));
                    if first_point.is_none() {
                        first_point = Some([x, y]);
                    } else {
                        last_point = Some([x, y]);
                    }
                }
                None => {
                    // Packet sent but not received
                    if !segment.is_empty() {
                        segments.push(segment);
                        segment = Vec::new();
                    }
                }
            }
            if break_after {
                // Add the first point that is outside the rectangle to complete the line, then break
                break;
            }
        }
        if !segment.is_empty() {
            segments.push(segment);
        }
        if segments.len() > state.ids.paths.len() {
            let mut id_gen = ui.widget_id_generator();
            state.update(|state| state.ids.paths.resize(segments.len(), &mut id_gen));
        }
        if points.len() > state.ids.points.len() {
            let mut id_gen = ui.widget_id_generator();
            state.update(|state| state.ids.points.resize(points.len(), &mut id_gen));
        }

        if log_enabled!(Level::Debug) {
            trace!(
                "Updating ringbuf widget with {} points in {} segments. First and last points: {:?} / {:?}",
                points.len(),
                segments.len(),
                first_point,
                last_point
            );
        }

        let segments = segments // Assign widget IDs to segments
            .iter()
            .enumerate()
            .map(|(i, segment)| (state.ids.paths[i], segment))
            .collect::<Vec<_>>();

        let thickness = self.style.line_thickness(ui.theme());
        let color = self.style.color(ui.theme());
        for (subid, segment) in segments.iter() {
            if segment.len() > 1 {
                widget::PointPath::new(segment.iter().map(|p| p.1))
                    .wh(rect.dim())
                    .xy(rect.xy())
                    .color(color)
                    .thickness(thickness)
                    .parent(id)
                    .graphics_for(id)
                    .set(*subid, ui);
            }
        }

        let radius = self.style.point_thickness(ui.theme()) / 2.;
        for (id, (_, point)) in points.iter().enumerate() {
            widget::primitive::shape::circle::Circle::fill(radius)
                .xy(*point)
                .color(RED)
                .set(state.ids.points[id], ui);
        }
    }
}

impl Colorable for LatencyGraphWidget<'_> {
    builder_method!(color { style.color = Some(Color) });
}
