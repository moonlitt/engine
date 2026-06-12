//! Bevy + moonlitt piano-tiles demo.
//!
//! Purpose: validate that moonlitt can be embedded in a real-time game loop
//! with no FFI/binding friction. Press D F J K to play C D E F.

use bevy::prelude::*;
use moonlitt_audio_io::Runtime;

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";
const SAMPLE_RATE: u32 = 44100;
const BUFFER_SIZE: u32 = 256;

const COLUMNS: usize = 4;
const COLUMN_X: [f32; COLUMNS] = [-150.0, -50.0, 50.0, 150.0];
const COLUMN_NOTES: [u8; COLUMNS] = [60, 62, 64, 65]; // C D E F
const COLUMN_KEYS: [KeyCode; COLUMNS] = [
    KeyCode::KeyD,
    KeyCode::KeyF,
    KeyCode::KeyJ,
    KeyCode::KeyK,
];
const COLUMN_COLORS: [(f32, f32, f32); COLUMNS] = [
    (0.31, 0.76, 0.97),
    (0.51, 0.78, 0.52),
    (1.00, 0.72, 0.30),
    (0.94, 0.32, 0.31),
];

const HIT_LINE_Y: f32 = -250.0;
const TILE_SPAWN_Y: f32 = 400.0;
const TILE_FALL_SPEED: f32 = 250.0;
const HIT_TOLERANCE: f32 = 50.0;
const TILE_SIZE: Vec2 = Vec2::new(80.0, 60.0);
const COLUMN_WIDTH: f32 = 100.0;

const MELODY: &[(usize, f32)] = &[
    (0, 0.0), (1, 0.5), (2, 1.0), (3, 1.5),
    (3, 2.0), (2, 2.5), (1, 3.0), (0, 3.5),
    (0, 4.0), (0, 4.5), (1, 5.0), (2, 5.5),
    (3, 6.0), (3, 6.5), (2, 7.0), (0, 7.5),
];

struct MoonlittAudio(Runtime);

#[derive(Resource, Default)]
struct GameState {
    elapsed: f32,
    next_melody_idx: usize,
}

#[derive(Component)]
struct Tile {
    column: usize,
}

#[derive(Component)]
struct HitFlash {
    timer: Timer,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "moonlitt × bevy — piano tiles".into(),
                resolution: (640.0, 800.0).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(GameState::default())
        .insert_resource(ClearColor(Color::srgb(0.07, 0.07, 0.12)))
        .add_systems(Startup, (setup_scene, setup_audio))
        .add_systems(Update, (
            spawn_melody,
            fall_tiles,
            handle_input,
            despawn_offscreen,
            fade_hit_flash,
        ))
        .run();
}

fn setup_audio(world: &mut World) {
    let backend = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE)
        .expect("failed to load SF2 — check SF2_PATH");
    let runtime = Runtime::new(backend, SAMPLE_RATE, BUFFER_SIZE)
        .map_err(|(e, _)| e)
        .expect("failed to create Runtime");
    runtime.start().expect("failed to start audio");
    info!("moonlitt audio started: {} Hz, {} samples", SAMPLE_RATE, BUFFER_SIZE);
    world.insert_non_send_resource(MoonlittAudio(runtime));
}

fn setup_scene(mut commands: Commands) {
    commands.spawn(Camera2d);

    // Hit line
    commands.spawn((
        Sprite {
            color: Color::srgba(0.49, 0.30, 1.00, 0.6),
            custom_size: Some(Vec2::new(640.0, 4.0)),
            ..default()
        },
        Transform::from_xyz(0.0, HIT_LINE_Y, 0.0),
    ));

    // Column dividers
    for &x in &COLUMN_X {
        commands.spawn((
            Sprite {
                color: Color::srgba(0.16, 0.16, 0.25, 1.0),
                custom_size: Some(Vec2::new(2.0, 800.0)),
                ..default()
            },
            Transform::from_xyz(x - 50.0, 0.0, -1.0),
        ));
    }
    // Right edge of last column
    commands.spawn((
        Sprite {
            color: Color::srgba(0.16, 0.16, 0.25, 1.0),
            custom_size: Some(Vec2::new(2.0, 800.0)),
            ..default()
        },
        Transform::from_xyz(COLUMN_X[COLUMNS - 1] + 50.0, 0.0, -1.0),
    ));
}

fn spawn_melody(
    time: Res<Time>,
    mut state: ResMut<GameState>,
    mut commands: Commands,
) {
    state.elapsed += time.delta_secs();
    let travel_time = (TILE_SPAWN_Y - HIT_LINE_Y) / TILE_FALL_SPEED;
    while state.next_melody_idx < MELODY.len() {
        let (col, t) = MELODY[state.next_melody_idx];
        if state.elapsed >= t - travel_time {
            spawn_tile(&mut commands, col);
            state.next_melody_idx += 1;
        } else {
            break;
        }
    }
}

fn spawn_tile(commands: &mut Commands, column: usize) {
    let (r, g, b) = COLUMN_COLORS[column];
    commands.spawn((
        Sprite {
            color: Color::srgb(r, g, b),
            custom_size: Some(TILE_SIZE),
            ..default()
        },
        Transform::from_xyz(COLUMN_X[column], TILE_SPAWN_Y, 0.0),
        Tile { column },
    ));
}

fn fall_tiles(time: Res<Time>, mut q: Query<&mut Transform, With<Tile>>) {
    let dy = TILE_FALL_SPEED * time.delta_secs();
    for mut t in &mut q {
        t.translation.y -= dy;
    }
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut audio: NonSendMut<MoonlittAudio>,
    q: Query<(Entity, &Tile, &Transform)>,
    mut commands: Commands,
) {
    for (col, &key) in COLUMN_KEYS.iter().enumerate() {
        if keys.just_pressed(key) {
            audio.0.note_on(0, COLUMN_NOTES[col], 100);

            let mut hit_entity = None;
            let mut closest = HIT_TOLERANCE;
            for (e, tile, t) in &q {
                if tile.column != col {
                    continue;
                }
                let dist = (t.translation.y - HIT_LINE_Y).abs();
                if dist < closest {
                    closest = dist;
                    hit_entity = Some(e);
                }
            }

            if let Some(e) = hit_entity {
                commands.entity(e).despawn();
                spawn_hit_flash(&mut commands, col);
            }
        }
        if keys.just_released(key) {
            audio.0.note_off(0, COLUMN_NOTES[col]);
        }
    }
}

fn spawn_hit_flash(commands: &mut Commands, column: usize) {
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 1.0, 1.0),
            custom_size: Some(Vec2::new(COLUMN_WIDTH, 30.0)),
            ..default()
        },
        Transform::from_xyz(COLUMN_X[column], HIT_LINE_Y, 1.0),
        HitFlash {
            timer: Timer::from_seconds(0.2, TimerMode::Once),
        },
    ));
}

fn fade_hit_flash(
    time: Res<Time>,
    mut q: Query<(Entity, &mut Sprite, &mut HitFlash)>,
    mut commands: Commands,
) {
    for (e, mut sprite, mut flash) in &mut q {
        flash.timer.tick(time.delta());
        let remaining = flash.timer.fraction_remaining();
        sprite.color.set_alpha(remaining);
        if flash.timer.finished() {
            commands.entity(e).despawn();
        }
    }
}

fn despawn_offscreen(
    q: Query<(Entity, &Transform), With<Tile>>,
    mut commands: Commands,
) {
    for (e, t) in &q {
        if t.translation.y < HIT_LINE_Y - 100.0 {
            commands.entity(e).despawn();
        }
    }
}
