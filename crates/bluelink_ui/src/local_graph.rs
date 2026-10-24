//! Local graph: current note + 1-hop neighbors, drawn as a floating force-directed
//! simulation (egui painter, in-process, no workers).
//!
//! The simulation lives in world space (center note near the origin); a lerped
//! pan/zoom camera maps it to the screen. Repaints are requested only while the
//! graph is still moving, hovered, or the camera is animating.

use std::collections::HashMap;
use std::f32::consts::TAU;

use bluelink_core::LocalGraph;
use eframe::egui::{self, Align2, CursorIcon, Pos2, Stroke, Vec2};

use crate::theme;

pub const MAX_NODES: usize = 30;

const REST_LENGTH: f32 = 110.0;
const SPRING: f32 = 10.0;
const REPULSION: f32 = 600_000.0;
const GRAVITY: f32 = 1.2;
const CENTER_GRAVITY: f32 = 8.0;
/// Per-second velocity decay rate; slightly underdamped so settling feels springy.
const DAMPING: f32 = 3.5;
const MAX_SPEED: f32 = 800.0;
/// Peak acceleration of the idle drift that keeps nodes gently floating.
const DRIFT: f32 = 80.0;
const HEAT_COOL_SECONDS: f32 = 6.0;
/// Heat held while the pointer is over the panel, so hovering keeps the graph alive.
const HOVER_HEAT: f32 = 0.5;
const SLEEP_SPEED: f32 = 1.0;
const MAX_DT: f32 = 1.0 / 30.0;
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 6.0;
/// World radius the initial camera fits into the panel.
const FIT_RADIUS: f32 = 220.0;

struct Body {
    path: String,
    pos: Vec2,
    vel: Vec2,
}

pub struct LocalGraphUi {
    /// Same order as `LocalGraph::nodes` (center first).
    bodies: Vec<Body>,
    /// 0..1: how much the simulation is "awake"; scales idle drift, 0 lets it sleep.
    heat: f32,
    /// False once the last frame ended with everything still.
    moving: bool,
    pan: Vec2,
    target_pan: Vec2,
    zoom: f32,
    target_zoom: f32,
    fitted: bool,
    dragging: Option<usize>,
}

impl Default for LocalGraphUi {
    fn default() -> Self {
        Self {
            bodies: Vec::new(),
            heat: 1.0,
            moving: true,
            pan: Vec2::ZERO,
            target_pan: Vec2::ZERO,
            zoom: 1.0,
            target_zoom: 1.0,
            fitted: false,
            dragging: None,
        }
    }
}

/// Draw the local graph in the central panel. Returns a note path when the user clicks a neighbor.
pub fn show_panel(
    ui: &mut egui::Ui,
    state: &mut LocalGraphUi,
    graph: &LocalGraph,
    mut title_of: impl FnMut(&str) -> String,
) -> Option<String> {
    sync_bodies(state, graph);

    ui.label(
        egui::RichText::new(format!(
            "{} notes · click a neighbor to recenter · drag to move · scroll to zoom",
            graph.nodes.len()
        ))
        .size(13.0)
        .color(theme::WEAK),
    );
    ui.add_space(6.0);

    let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, theme::PANEL);
    let origin = rect.center();

    let index: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    let edges: Vec<(usize, usize)> = graph
        .edges
        .iter()
        .filter_map(|(a, b)| Some((*index.get(a.as_str())?, *index.get(b.as_str())?)))
        .collect();
    let radii = node_radii(graph.nodes.len(), &edges);

    if !state.fitted {
        let fit = ((rect.size().min_elem() * 0.5 - 40.0) / FIT_RADIUS).clamp(0.4, 1.2);
        state.zoom = fit;
        state.target_zoom = fit;
        state.fitted = true;
    }

    let dt = ui.input(|i| i.stable_dt).clamp(1e-3, MAX_DT);
    let time = ui.input(|i| i.time) as f32;

    // --- input ---
    if let Some(cursor) = resp.hover_pos() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        if scroll != 0.0 || pinch != 1.0 {
            zoom_toward(state, cursor - origin, (scroll * 0.002).exp() * pinch);
        }
    }

    if resp.drag_started() {
        let screen = screen_positions(state, origin);
        let press = ui.input(|i| i.pointer.press_origin());
        state.dragging = press.and_then(|p| node_at(&screen, &radii, p));
    }
    match (state.dragging, resp.interact_pointer_pos()) {
        (Some(i), Some(p)) => {
            state.bodies[i].pos = (p - origin - state.pan) / state.zoom;
            state.bodies[i].vel = Vec2::ZERO;
            state.heat = 1.0;
        }
        (None, _) if resp.dragged() => {
            state.pan += resp.drag_delta();
            state.target_pan += resp.drag_delta();
        }
        _ => {}
    }
    if resp.drag_stopped() {
        state.dragging = None;
    }

    // --- simulation ---
    state.heat = (state.heat - dt / HEAT_COOL_SECONDS).max(0.0);
    if resp.hovered() {
        state.heat = state.heat.max(HOVER_HEAT);
    }
    if state.heat > 0.0 || state.moving {
        let max_speed = step(
            &mut state.bodies,
            &edges,
            state.dragging,
            state.heat,
            time,
            dt,
        );
        state.moving = state.heat > 0.0 || max_speed >= SLEEP_SPEED;
        if !state.moving {
            state.bodies.iter_mut().for_each(|b| b.vel = Vec2::ZERO);
        }
    }

    let camera_moving = ease_camera(state, dt);

    // --- hover / click ---
    let screen = screen_positions(state, origin);
    let hovered = state
        .dragging
        .or_else(|| resp.hover_pos().and_then(|p| node_at(&screen, &radii, p)));

    if state.dragging.is_some() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    let clicked = if resp.clicked() {
        hovered
            .filter(|&i| i != 0)
            .map(|i| graph.nodes[i].clone())
    } else {
        None
    };

    // --- draw ---
    let lit = lit_nodes(graph.nodes.len(), &edges, hovered);
    draw_edges(&painter, &screen, &edges, hovered);

    let text_alpha = ((state.zoom - 0.45) / 0.35).clamp(0.0, 1.0);
    for (i, path) in graph.nodes.iter().enumerate() {
        let is_center = i == 0;
        let is_hovered = hovered == Some(i);
        let dim = if lit[i] { 1.0 } else { 0.22 };
        let base = if is_center { theme::ACCENT } else { theme::TEXT };
        let pos = screen[i];
        // Obsidian-style: node size grows with sqrt(zoom), not linearly.
        let r = radii[i] * state.zoom.sqrt();

        if is_center {
            for (extra, alpha) in [(16.0, 0.05), (10.0, 0.09), (5.0, 0.16)] {
                painter.circle_filled(pos, r + extra, theme::ACCENT.gamma_multiply(alpha * dim));
            }
        }
        painter.circle_filled(pos, r, base.gamma_multiply(dim));
        if is_hovered {
            painter.circle_stroke(pos, r + 4.0, Stroke::new(1.5_f32, theme::ACCENT));
        }

        let label_alpha = if is_center || is_hovered { 1.0 } else { text_alpha * dim };
        if label_alpha > 0.02 {
            let size = if is_center { 14.0 } else { 12.0 };
            painter.text(
                pos + Vec2::new(0.0, r + 5.0),
                Align2::CENTER_TOP,
                title_of(path),
                egui::FontId::proportional(size),
                base.gamma_multiply(label_alpha),
            );
        }
    }

    if state.moving || camera_moving {
        ui.ctx().request_repaint();
    }

    clicked
}

/// Keep `bodies` aligned with `graph.nodes`: surviving nodes keep their motion,
/// new ones spawn at the old center and fly out, so a recenter animates.
fn sync_bodies(state: &mut LocalGraphUi, graph: &LocalGraph) {
    let unchanged = state.bodies.len() == graph.nodes.len()
        && state.bodies.iter().zip(&graph.nodes).all(|(b, n)| &b.path == n);
    if unchanged {
        return;
    }

    let anchor = state.bodies.first().map_or(Vec2::ZERO, |b| b.pos);
    let mut old: HashMap<String, Body> = std::mem::take(&mut state.bodies)
        .into_iter()
        .map(|b| (b.path.clone(), b))
        .collect();
    let count = graph.nodes.len().max(1) as f32;

    state.bodies = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, path)| {
            old.remove(path).unwrap_or_else(|| Body {
                path: path.clone(),
                pos: anchor + Vec2::angled(TAU * i as f32 / count) * 20.0,
                vel: Vec2::ZERO,
            })
        })
        .collect();
    state.dragging = None;
    state.heat = 1.0;
    state.moving = true;
}

/// Node radius by degree (capped), in world units.
fn node_radii(n: usize, edges: &[(usize, usize)]) -> Vec<f32> {
    let mut degree = vec![0usize; n];
    for &(a, b) in edges {
        degree[a] += 1;
        degree[b] += 1;
    }
    degree
        .iter()
        .map(|&d| (3.5 + (d as f32).sqrt() * 1.4).min(9.0))
        .collect()
}

fn screen_positions(state: &LocalGraphUi, origin: Pos2) -> Vec<Pos2> {
    state
        .bodies
        .iter()
        .map(|b| origin + state.pan + b.pos * state.zoom)
        .collect()
}

fn node_at(screen: &[Pos2], radii: &[f32], p: Pos2) -> Option<usize> {
    screen
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.distance(p)))
        .filter(|&(i, d)| d <= radii[i] + 8.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Nodes that stay bright: everything when nothing is hovered, else hovered + neighbors.
fn lit_nodes(n: usize, edges: &[(usize, usize)], hovered: Option<usize>) -> Vec<bool> {
    let Some(h) = hovered else {
        return vec![true; n];
    };
    let mut lit = vec![false; n];
    lit[h] = true;
    for &(a, b) in edges {
        if a == h {
            lit[b] = true;
        }
        if b == h {
            lit[a] = true;
        }
    }
    lit
}

fn draw_edges(
    painter: &egui::Painter,
    screen: &[Pos2],
    edges: &[(usize, usize)],
    hovered: Option<usize>,
) {
    for &(a, b) in edges {
        let incident = hovered.is_some_and(|h| h == a || h == b);
        let stroke = match (hovered, incident) {
            (Some(_), true) => Stroke::new(1.8_f32, theme::ACCENT.gamma_multiply(0.9)),
            (Some(_), false) => Stroke::new(1.0_f32, theme::STROKE.gamma_multiply(0.15)),
            (None, _) => Stroke::new(1.0_f32, theme::STROKE.gamma_multiply(0.55)),
        };
        painter.line_segment([screen[a], screen[b]], stroke);
    }
}

/// Zoom so the world point under `cursor` (relative to panel center) stays put.
fn zoom_toward(state: &mut LocalGraphUi, cursor: Vec2, factor: f32) {
    let world = (cursor - state.target_pan) / state.target_zoom;
    state.target_zoom = (state.target_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    state.target_pan = cursor - world * state.target_zoom;
}

/// Lerp camera toward its targets. Returns true while it is still moving.
fn ease_camera(state: &mut LocalGraphUi, dt: f32) -> bool {
    let k = 1.0 - (-14.0 * dt).exp();
    state.zoom += (state.target_zoom - state.zoom) * k;
    state.pan += (state.target_pan - state.pan) * k;

    let zoom_left = (state.target_zoom - state.zoom).abs();
    let pan_left = (state.target_pan - state.pan).length();
    if zoom_left < 1e-3 && pan_left < 0.1 {
        state.zoom = state.target_zoom;
        state.pan = state.target_pan;
        return false;
    }
    true
}

/// One physics step (O(n²), fine under MAX_NODES). The pinned body (being dragged)
/// is not integrated but still pushes and pulls its neighbors.
/// Returns the max body speed after the step.
// ponytail: O(n²) repulsion — upgrade to Barnes–Hut if MAX_NODES grows by an order of magnitude.
fn step(
    bodies: &mut [Body],
    edges: &[(usize, usize)],
    pinned: Option<usize>,
    heat: f32,
    time: f32,
    dt: f32,
) -> f32 {
    let n = bodies.len();
    let mut acc = vec![Vec2::ZERO; n];

    for i in 0..n {
        for j in (i + 1)..n {
            let delta = bodies[i].pos - bodies[j].pos;
            let len = delta.length();
            let dir = if len > 1e-3 {
                delta / len
            } else {
                Vec2::angled((i + j) as f32)
            };
            // Floor the distance so near-overlaps push hard but never explode.
            let push = dir * (REPULSION / (len * len).max(400.0));
            acc[i] += push;
            acc[j] -= push;
        }
    }

    for &(a, b) in edges {
        let delta = bodies[b].pos - bodies[a].pos;
        let len = delta.length().max(1e-3);
        let pull = delta / len * ((len - REST_LENGTH) * SPRING);
        acc[a] += pull;
        acc[b] -= pull;
    }

    let damp = (-DAMPING * dt).exp();
    let mut max_speed = 0.0_f32;
    for (i, body) in bodies.iter_mut().enumerate() {
        if pinned == Some(i) {
            body.vel = Vec2::ZERO;
            continue;
        }
        let gravity = if i == 0 { CENTER_GRAVITY } else { GRAVITY };
        let phase = i as f32 * 1.9;
        let drift = Vec2::new((time * 0.7 + phase).sin(), (time * 0.53 + phase * 1.3).cos())
            * (DRIFT * heat);

        body.vel += (acc[i] - body.pos * gravity + drift) * dt;
        body.vel *= damp;
        let speed = body.vel.length();
        if speed > MAX_SPEED {
            body.vel *= MAX_SPEED / speed;
        }
        body.pos += body.vel * dt;
        max_speed = max_speed.max(body.vel.length());
    }
    max_speed
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluelink_core::WikiIndex;

    fn bodies_for(graph: &LocalGraph) -> Vec<Body> {
        let mut state = LocalGraphUi::default();
        sync_bodies(&mut state, graph);
        state.bodies
    }

    #[test]
    fn sync_creates_a_body_per_node_and_keeps_survivors() {
        let mut i = WikiIndex::new();
        i.set_note("a.md", "[[b]] [[c]]");
        i.set_note("b.md", "");
        i.set_note("c.md", "[[a]]");
        let g = i.local_graph("a.md", MAX_NODES);

        let mut state = LocalGraphUi::default();
        sync_bodies(&mut state, &g);
        assert_eq!(state.bodies.len(), g.nodes.len());

        state.bodies[1].pos = Vec2::new(55.0, 5.0);
        let moved = state.bodies[1].path.clone();
        sync_bodies(&mut state, &i.local_graph("b.md", MAX_NODES));
        let kept = state.bodies.iter().find(|b| b.path == moved).unwrap();
        assert_eq!(kept.pos, Vec2::new(55.0, 5.0));
    }

    #[test]
    fn step_stays_finite_and_bounded_even_with_coincident_nodes_and_a_pinned_drag() {
        let mut i = WikiIndex::new();
        let links: String = (0..29).map(|n| format!("[[n{n}]] ")).collect();
        i.set_note("hub.md", &links);
        for n in 0..29 {
            i.set_note(&format!("n{n}.md"), "[[hub]]");
        }
        let g = i.local_graph("hub.md", MAX_NODES);
        let index: HashMap<&str, usize> =
            g.nodes.iter().enumerate().map(|(i, p)| (p.as_str(), i)).collect();
        let edges: Vec<(usize, usize)> = g
            .edges
            .iter()
            .map(|(a, b)| (index[a.as_str()], index[b.as_str()]))
            .collect();

        let mut bodies = bodies_for(&g);
        // Worst case: everything starts on the same point.
        bodies.iter_mut().for_each(|b| b.pos = Vec2::ZERO);

        let mut speed = f32::MAX;
        for frame in 0..1200 {
            let pinned = (frame < 600).then_some(3);
            if pinned.is_some() {
                bodies[3].pos = Vec2::new(300.0, -200.0); // drag target
            }
            speed = step(&mut bodies, &edges, pinned, 1.0, frame as f32 / 60.0, MAX_DT);
            assert!(bodies.iter().all(|b| b.pos.is_finite() && b.vel.is_finite()));
            assert!(bodies.iter().all(|b| b.pos.length() < 2000.0));
        }
        assert!(speed < 200.0, "drift should stay gentle, got {speed}");
    }
}
