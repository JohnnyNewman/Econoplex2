//! World rendering: towns, buildings, nature, agents, camera, selection.

use crate::Selection;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::components::{Agent, NaturalResource, Pos, Product, Structure};
use sim_core::store::Activity;
use sim_core::world::Params;
use sim_core::{AgentStore, Db, Towns};

/// Toggleable overlays.
#[derive(Resource, Default)]
pub struct Overlay {
    pub product_space: bool,
}

/// Skill name label for the product space overlay.
#[derive(Component)]
pub struct SkillLabel(pub usize);

/// Screen-independent layout of the product space: skills on a circle, grouped by domain.
pub fn product_space_layout(db: &Db, center: Vec2, radius: f32) -> Vec<Vec2> {
    let ns = db.content.skills.len();
    let mut order: Vec<usize> = (0..ns).collect();
    order.sort_by_key(|&s| (db.content.domain(&db.content.skills[s].domain), s));
    let mut p = vec![Vec2::ZERO; ns];
    for (k, &s) in order.iter().enumerate() {
        let a = k as f32 / ns as f32 * std::f32::consts::TAU;
        p[s] = center + radius * Vec2::new(a.cos(), a.sin());
    }
    p
}

pub fn spawn_skill_labels(mut commands: Commands, db: Res<Db>) {
    for (i, s) in db.content.skills.iter().enumerate() {
        commands.spawn((
            SkillLabel(i),
            Text2d::new(s.name.clone()),
            TextFont {
                font_size: FontSize::Px(13.0),
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.95, 0.95)),
            Transform::from_xyz(0.0, 0.0, 10.0),
            Visibility::Hidden,
        ));
    }
}

/// Keep labels next to their nodes (outside the circle) at constant screen size.
pub fn update_skill_labels(
    db: Res<Db>,
    overlay: Res<Overlay>,
    camera: Single<&Transform, (With<Camera2d>, Without<SkillLabel>)>,
    mut labels: Query<(&SkillLabel, &mut Transform, &mut Visibility)>,
) {
    let center = camera.translation.truncate();
    let zoom = camera.scale.x;
    let pos = product_space_layout(&db, center, 220.0 * zoom);
    for (l, mut tf, mut vis) in &mut labels {
        *vis = if overlay.product_space {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let dir = (pos[l.0] - center).normalize_or_zero();
        let at = pos[l.0] + dir * 48.0 * zoom;
        tf.translation = at.extend(10.0);
        tf.scale = Vec3::splat(zoom);
    }
}

/// Visual state of an agent sprite.
#[derive(Component)]
pub struct AgentSprite;

/// Color for a skill domain: evenly spaced hues.
pub fn domain_color(domain: usize, n: usize, strength: f32) -> Color {
    let hue = 360.0 * domain as f32 / n.max(1) as f32;
    Color::hsl(hue, 0.35 + 0.45 * strength, 0.35 + 0.3 * strength)
}

/// The agent's best domain and how good they are at it (0..1).
pub fn dominant_domain(db: &Db, cap: &sim_data::dims::CapVec) -> (usize, f32) {
    let mut best = (0, f32::MIN);
    for d in 0..db.domain_dirs.len() {
        let p = db.domain_proficiency(cap, d);
        if p > best.1 {
            best = (d, p);
        }
    }
    (best.0, (best.1 / 1.5).clamp(0.0, 1.0))
}

pub fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Transform::from_scale(Vec3::new(1.6, 1.6, 1.0))));
}

/// Sprites and labels for buildings, including ones constructed during play.
pub fn attach_building_sprites(
    mut commands: Commands,
    db: Res<Db>,
    buildings: Query<(Entity, &Product, &Pos), Added<Structure>>,
) {
    for (e, product, pos) in &buildings {
        commands.entity(e).insert((
            Sprite::from_color(Color::srgb(0.55, 0.42, 0.3), Vec2::splat(26.0)),
            Transform::from_xyz(pos.x, pos.y, 1.0),
        ));
        commands.spawn((
            Text2d::new(db.content.products[product.def as usize].name.clone()),
            TextFont {
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.8, 0.7)),
            Transform::from_xyz(pos.x, pos.y - 22.0, 5.0),
        ));
    }
}

pub fn spawn_static(mut commands: Commands, towns: Res<Towns>) {
    for town in &towns.0 {
        commands.spawn((
            Text2d::new(town.name.clone()),
            TextFont {
                font_size: FontSize::Px(22.0),
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.92, 0.8)),
            Transform::from_xyz(town.pos.0, town.pos.1 + 120.0, 5.0),
        ));
    }
}

/// Nature sites get a colored square: at startup, and when a town opens new land.
pub fn attach_nature_sprites(
    mut commands: Commands,
    db: Res<Db>,
    nature: Query<(Entity, &NaturalResource, &Pos), Without<Sprite>>,
) {
    for (e, n, pos) in &nature {
        let color = match db.content.nature[n.kind as usize].id.as_str() {
            "forest" => Color::srgb(0.15, 0.45, 0.2),
            "field" => Color::srgb(0.75, 0.65, 0.25),
            "ore_vein" => Color::srgb(0.5, 0.5, 0.55),
            _ => Color::srgb(0.4, 0.4, 0.4),
        };
        commands.entity(e).insert((
            Sprite::from_color(color, Vec2::splat(18.0)),
            Transform::from_xyz(pos.x, pos.y, 0.5),
        ));
    }
}

/// Give newly born (or newly spawned) agents a sprite.
pub fn attach_agent_sprites(
    mut commands: Commands,
    store: Res<AgentStore>,
    added: Query<(Entity, &Agent), Without<AgentSprite>>,
) {
    for (e, a) in &added {
        let (x, y) = store.target[a.idx as usize];
        commands.entity(e).insert((
            AgentSprite,
            Sprite::from_color(Color::WHITE, Vec2::splat(5.0)),
            Transform::from_xyz(x, y, 2.0),
        ));
    }
}

/// Move agents toward their targets and color them by their best skill domain.
pub fn animate_agents(
    time: Res<Time>,
    store: Res<AgentStore>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<sim_core::SimClock>,
    selection: Res<Selection>,
    mut agents: Query<(&Agent, &mut Transform, &mut Sprite), With<AgentSprite>>,
) {
    let dt = time.delta_secs();
    let n = db.domain_dirs.len();
    for (a, mut tf, mut sprite) in &mut agents {
        let i = a.idx as usize;
        let target = Vec2::new(store.target[i].0, store.target[i].1);
        let pos = tf.translation.truncate();
        let delta = target - pos;
        let step = 160.0 * dt;
        let next = if delta.length() <= step {
            target
        } else {
            pos + delta.normalize() * step
        };
        tf.translation.x = next.x;
        tf.translation.y = next.y;

        let child = store.age_years(i, clock.days_per_year) < params.life.adult_age;
        let selected = selection.0 == Some(a.idx);
        let (d, strength) = dominant_domain(&db, &store.cap[i]);
        sprite.color = if selected {
            Color::WHITE
        } else if child {
            Color::srgb(0.75, 0.75, 0.75)
        } else {
            domain_color(d, n, strength)
        };
        let size = if selected {
            10.0
        } else if child {
            3.5
        } else {
            5.0
        };
        sprite.custom_size = Some(Vec2::splat(size));
        tf.translation.z = if selected { 4.0 } else { 2.0 };
    }
}

pub fn draw_world(
    mut gizmos: Gizmos,
    towns: Res<Towns>,
    db: Res<Db>,
    store: Res<AgentStore>,
    selection: Res<Selection>,
    overlay: Res<Overlay>,
    nature: Query<(&NaturalResource, &Pos)>,
    camera: Single<&Transform, With<Camera2d>>,
) {
    for town in &towns.0 {
        gizmos.circle_2d(
            Vec2::new(town.pos.0, town.pos.1),
            100.0,
            Color::srgba(0.9, 0.85, 0.7, 0.25),
        );
    }
    // Resource level of each nature site as a ring.
    for (n, pos) in &nature {
        let level = (n.amount / n.capacity).clamp(0.0, 1.0);
        gizmos.circle_2d(
            Vec2::new(pos.x, pos.y),
            6.0 + 10.0 * level,
            Color::srgba(1.0, 1.0, 1.0, 0.15 + 0.25 * level),
        );
    }
    if let Some(i) = selection.0 {
        let i = i as usize;
        if store.alive[i] {
            let (x, y) = store.target[i];
            gizmos.circle_2d(Vec2::new(x, y), 12.0, Color::WHITE);
            if let Some(p) = store.partner[i] {
                let (px, py) = store.target[p as usize];
                gizmos.line_2d(
                    Vec2::new(x, y),
                    Vec2::new(px, py),
                    Color::srgba(1.0, 0.5, 0.7, 0.6),
                );
            }
        }
    }
    if overlay.product_space {
        draw_product_space(
            &mut gizmos,
            &db,
            &store,
            selection.0,
            camera.translation.truncate(),
            camera.scale.x,
        );
    }
}

/// The skill graph laid out on a circle grouped by domain; edges = embedding similarity,
/// node brightness = the selected agent's proficiency (or the population mean).
fn draw_product_space(
    gizmos: &mut Gizmos,
    db: &Db,
    store: &AgentStore,
    selected: Option<u32>,
    center: Vec2,
    zoom: f32,
) {
    let ns = db.content.skills.len();
    let pos = product_space_layout(db, center, 220.0 * zoom);
    for a in 0..ns {
        for b in (a + 1)..ns {
            let sim = db.embedding.similarity(a, b);
            if sim > 0.2 {
                gizmos.line_2d(
                    pos[a],
                    pos[b],
                    Color::srgba(0.8, 0.8, 0.9, (sim - 0.15) * 0.8),
                );
            }
        }
    }
    let profile: Vec<f32> = match selected {
        Some(i) if store.alive[i as usize] => db.skill_profile(&store.cap[i as usize]),
        _ => {
            let mut sum = vec![0.0; ns];
            let mut n: f32 = 0.0;
            for i in 0..store.len() {
                if store.alive[i] {
                    for (s, v) in db.skill_profile(&store.cap[i]).iter().enumerate() {
                        sum[s] += v;
                    }
                    n += 1.0;
                }
            }
            sum.iter().map(|v| v / n.max(1.0)).collect()
        }
    };
    let nd = db.domain_dirs.len();
    for s in 0..ns {
        let d = db.content.domain(&db.content.skills[s].domain).unwrap() as usize;
        let level = (profile[s] / 1.5).clamp(0.0, 1.0);
        gizmos.circle_2d(
            pos[s],
            (6.0 + 14.0 * level) * zoom,
            domain_color(d, nd, level),
        );
    }
}

pub fn camera_controls(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut camera: Single<&mut Transform, With<Camera2d>>,
) {
    let mut dir = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        dir.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        dir.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        dir.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        dir.x += 1.0;
    }
    let zoom = camera.scale.x;
    camera.translation += (dir * 600.0 * zoom * time.delta_secs()).extend(0.0);
    if scroll.delta.y != 0.0 {
        let z = (zoom * (1.0 - 0.1 * scroll.delta.y.signum())).clamp(0.2, 5.0);
        camera.scale = Vec3::new(z, z, 1.0);
    }
}

pub fn select_agent(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    agents: Query<(&Agent, &Transform), With<AgentSprite>>,
    mut selection: ResMut<Selection>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let (cam, cam_tf) = *camera;
    let Ok(world) = cam.viewport_to_world_2d(cam_tf, cursor) else {
        return;
    };
    let zoom = cam_tf.scale().x;
    let mut best: Option<(u32, f32)> = None;
    for (a, tf) in &agents {
        let d = tf.translation.truncate().distance(world);
        if d < 12.0 * zoom && best.is_none_or(|b| d < b.1) {
            best = Some((a.idx, d));
        }
    }
    if let Some((idx, _)) = best {
        selection.0 = Some(idx);
    }
}

/// Short description of what an agent is doing.
pub fn activity_label(
    store: &AgentStore,
    db: &Db,
    tasks: &Query<&sim_core::components::Task>,
    i: usize,
) -> String {
    match store.activity[i] {
        Activity::Child => "child".into(),
        Activity::Idle => "idle".into(),
        Activity::Resting => "resting".into(),
        Activity::Working(e) => match tasks.get(e) {
            Ok(t) => format!("working: {}", db.recipes[t.recipe as usize].name),
            Err(_) => "working".into(),
        },
        Activity::Captive => "captive".into(),
        Activity::Soldier => match store.weapon[i] {
            Some(_) => "soldier (armed)".into(),
            None => "soldier".into(),
        },
    }
}
