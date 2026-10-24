//! Minimal local graph: current note + 1-hop neighbors (no PIXI / workers).

use std::collections::HashMap;
use std::f32::consts::TAU;

use bluelink_core::LocalGraph;
use eframe::egui::{self, Pos2, Rect, Vec2};

use crate::theme;

pub const MAX_NODES: usize = 30;

#[derive(Default)]
pub struct LocalGraphUi {
    /// Center path the current positions were seeded for.
    layout_center: Option<String>,
    positions: HashMap<String, Vec2>,
}

/// Draw the local graph in the central panel. Returns a note path when the user clicks a neighbor.
pub fn show_panel(
    ui: &mut egui::Ui,
    state: &mut LocalGraphUi,
    graph: &LocalGraph,
    mut title_of: impl FnMut(&str) -> String,
) -> Option<String> {
    ensure_layout(state, graph);

    let mut clicked: Option<String> = None;

    ui.label(
        egui::RichText::new(format!(
            "{} notes · click a neighbor to recenter",
            graph.nodes.len()
        ))
        .size(13.0)
        .color(theme::WEAK),
    );
    ui.add_space(6.0);

    let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, theme::PANEL);

    let positions = screen_positions(state, graph, rect);
    let stroke = egui::Stroke::new(1.0_f32, theme::STROKE);

    for (from, to) in &graph.edges {
        let Some(a) = positions.get(from.as_str()) else {
            continue;
        };
        let Some(b) = positions.get(to.as_str()) else {
            continue;
        };
        painter.line_segment([*a, *b], stroke);
    }

    for path in &graph.nodes {
        let Some(&pos) = positions.get(path.as_str()) else {
            continue;
        };
        let is_center = path == &graph.center;
        let label = title_of(path);
        let color = if is_center {
            theme::ACCENT
        } else {
            theme::TEXT
        };
        let font = egui::FontId::proportional(if is_center { 15.0 } else { 13.0 });
        let galley = painter.layout_no_wrap(label, font, color);
        let text_pos = pos - galley.size() * 0.5;
        let hit = Rect::from_min_size(text_pos, galley.size()).expand(4.0);

        painter.circle_filled(pos, if is_center { 4.0 } else { 3.0 }, color);
        painter.galley(text_pos, galley, color);

        if resp.clicked() && hit.contains(resp.interact_pointer_pos().unwrap_or(pos)) {
            if path != &graph.center {
                clicked = Some(path.clone());
            }
        }
    }

    clicked
}

fn ensure_layout(state: &mut LocalGraphUi, graph: &LocalGraph) {
    let same_center = state.layout_center.as_deref() == Some(graph.center.as_str());
    let same_nodes = same_center
        && graph.nodes.len() == state.positions.len()
        && graph.nodes.iter().all(|n| state.positions.contains_key(n));
    if same_nodes {
        return;
    }

    state.layout_center = Some(graph.center.clone());
    state.positions = spring_layout(graph);
}

fn spring_layout(graph: &LocalGraph) -> HashMap<String, Vec2> {
    let n = graph.nodes.len().max(1);
    let mut pos: HashMap<String, Vec2> = HashMap::with_capacity(n);

    // Seed: center at origin, neighbors on a ring (stable by node order).
    pos.insert(graph.center.clone(), Vec2::ZERO);
    let ring: Vec<&String> = graph
        .nodes
        .iter()
        .filter(|p| p.as_str() != graph.center.as_str())
        .collect();
    let radius = 120.0_f32;
    for (i, path) in ring.iter().enumerate() {
        let angle = TAU * (i as f32) / (ring.len().max(1) as f32);
        pos.insert(
            (*path).clone(),
            Vec2::new(angle.cos() * radius, angle.sin() * radius),
        );
    }

    // ponytail: O(n²) force layout — fine under MAX_NODES (~30); upgrade to Barnes–Hut if capped higher.
    let iterations = 48;
    let repulsion = 2800.0_f32;
    let spring = 0.045_f32;
    let rest = 110.0_f32;
    let damping = 0.85_f32;

    for _ in 0..iterations {
        let mut force: HashMap<String, Vec2> =
            graph.nodes.iter().map(|p| (p.clone(), Vec2::ZERO)).collect();

        for i in 0..graph.nodes.len() {
            for j in (i + 1)..graph.nodes.len() {
                let a = &graph.nodes[i];
                let b = &graph.nodes[j];
                let pa = pos[a];
                let pb = pos[b];
                let delta = pa - pb;
                let dist = delta.length().max(8.0);
                let dir = delta / dist;
                let push = dir * (repulsion / (dist * dist));
                *force.get_mut(a).unwrap() += push;
                *force.get_mut(b).unwrap() -= push;
            }
        }

        for (from, to) in &graph.edges {
            let pa = pos[from];
            let pb = pos[to];
            let delta = pb - pa;
            let dist = delta.length().max(1.0);
            let dir = delta / dist;
            let pull = dir * ((dist - rest) * spring);
            *force.get_mut(from).unwrap() += pull;
            *force.get_mut(to).unwrap() -= pull;
        }

        // Pin center.
        force.insert(graph.center.clone(), Vec2::ZERO);

        for path in &graph.nodes {
            if path == &graph.center {
                continue;
            }
            let f = force[path] * damping;
            let p = pos.get_mut(path).unwrap();
            let len = f.length();
            *p += if len > 24.0 { f * (24.0 / len) } else { f };
        }
    }

    pos
}

fn screen_positions<'a>(
    state: &'a LocalGraphUi,
    graph: &'a LocalGraph,
    rect: Rect,
) -> HashMap<&'a str, Pos2> {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for path in &graph.nodes {
        let Some(p) = state.positions.get(path) else {
            continue;
        };
        min = min.min(*p);
        max = max.max(*p);
    }
    if !min.is_finite() {
        return HashMap::new();
    }

    let span = (max - min).max(Vec2::splat(1.0));
    let pad = 36.0;
    let inner = Rect::from_min_max(
        rect.min + Vec2::splat(pad),
        rect.max - Vec2::splat(pad),
    );
    let scale = (inner.width() / span.x)
        .min(inner.height() / span.y)
        .min(1.4);

    let mut out = HashMap::with_capacity(graph.nodes.len());
    for path in &graph.nodes {
        let Some(p) = state.positions.get(path) else {
            continue;
        };
        let local = (*p - min - span * 0.5) * scale;
        out.insert(path.as_str(), inner.center() + local);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluelink_core::WikiIndex;

    #[test]
    fn spring_layout_places_every_node() {
        let mut i = WikiIndex::new();
        i.set_note("a.md", "[[b]] [[c]]");
        i.set_note("b.md", "");
        i.set_note("c.md", "[[a]]");
        let g = i.local_graph("a.md", MAX_NODES);
        let pos = spring_layout(&g);
        assert_eq!(pos.len(), g.nodes.len());
        assert_eq!(pos["a.md"], Vec2::ZERO);
    }
}
