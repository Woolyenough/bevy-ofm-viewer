use std::f64::consts::PI;

use bevy::app::*;
use bevy::core_pipeline::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_pancam::DirectionKeys;
use bevy_pancam::PanCam;
use bevy_pancam::PanCamPlugin;
use geo::scale;
use ofm_api::display_ofm_tile;
use ofm_api::get_ofm_data;
use ofm_api::tile_width_meters;
use ofm_api::OfmTiles;
use ofm_api::Tile;
use rstar::RTree;
use tile::Coord;
use tile_map::TileMapPlugin;

pub mod ofm_api;
pub mod tile;
pub mod tile_map;

pub const STARTING_LONG_LAT: Coord = Coord::new(52.18492, 0.14281721);
pub const STARTING_ZOOM: u32 = 14;
pub const TILE_QUALITY: i32 = 512;

fn main() {
    App::new()
    .add_plugins((DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "OFM Viewer".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    }), PanCamPlugin, TileMapPlugin))
    .add_systems(Startup, setup_camera)
    .add_systems(Update, (handle_mouse, display_ofm_tile))
    .insert_resource(OfmTiles {
        tiles: RTree::new(),
        tiles_to_render: Vec::new(),
    })
    .insert_resource(ClearColor(Color::from(Srgba { red: 0.1, green: 0.1, blue: 0.1, alpha: 1.0 })))
    .run();
}

pub fn handle_mouse(
    buttons: Res<ButtonInput<MouseButton>>,
    q_windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let (camera, camera_transform) = camera.single();

    if buttons.just_pressed(MouseButton::Left) {
        if let Some(position) = q_windows.single().cursor_position() {
            let world_pos = camera.viewport_to_world_2d(camera_transform, position).unwrap();
            info!("{:?}", world_mercator_to_lat_lon(world_pos.x.into(), world_pos.y.into(), STARTING_LONG_LAT));
        }
    } 

}

pub fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            hdr: true, // HDR is required for the bloom effect
            ..default()
        },
        PanCam {
            grab_buttons: vec![MouseButton::Middle], // which buttons should drag the camera
            move_keys: DirectionKeys {      // the keyboard buttons used to move the camera
                up:    vec![KeyCode::ArrowUp], // initalize the struct like this or use the provided methods for
                down:  vec![KeyCode::ArrowDown], // common key combinations
                left:  vec![KeyCode::ArrowLeft],
                right: vec![KeyCode::ArrowRight],
            },
            speed: 400., // the speed for the keyboard movement
            enabled: true, // when false, controls are disabled. See toggle example.
            zoom_to_cursor: true, // whether to zoom towards the mouse or the center of the screen
            min_scale: 0.25, // prevent the camera from zooming too far in
            max_scale: f32::INFINITY, // prevent the camera from zooming too far out
            min_x: f32::NEG_INFINITY, // minimum x position of the camera window
            max_x: f32::INFINITY, // maximum x position of the camera window
            min_y: f32::NEG_INFINITY, // minimum y position of the camera window
            max_y: f32::INFINITY, // maximum y position of the camera window
        },
        Bloom::NATURAL,
    ));
}

pub fn camera_space_to_lat_long_rect(
    transform: &GlobalTransform,
    window: &Window,
    projection: OrthographicProjection,
) -> Option<geo::Rect<f64>> {
    // Get the window size
    let window_width = window.width(); 
    let window_height = window.height();

    // Get the camera's position
    let camera_translation = transform.translation();

    // Compute the world-space rectangle
    // The reason for not dividing by 2 is to make the rectangle larger, as then it will mean that we can load more data
    let left = camera_translation.x ;
    let right = camera_translation.x  + ((window_width * projection.scale) / 2.0);
    let bottom = camera_translation.y + ((window_height * projection.scale) / 2.0);
    let top = camera_translation.y;
    
    Some(geo::Rect::new(
        world_mercator_to_lat_lon(left.into(), bottom.into(), STARTING_LONG_LAT),
        world_mercator_to_lat_lon(right.into(), top.into(), STARTING_LONG_LAT),
    ))
}


pub fn level_to_tile_width(level: i32) -> f32 {
    360.0 / (2_i32.pow(level as u32) as f32)
}

pub fn world_degreese_to_world_mercator(lon: f32) -> u32 {
    (lon * 20037508.34 / 180.0 ) as u32
}

pub fn geo_to_tile(lon_deg: f64, lat_deg: f64, zoom: u32) -> (i32, i32) {
    let n = (1 << zoom) as f64;

    let x_tile = (n * (lon_deg + 180.0) / 360.0) as i32;

    let lat_rad = lat_deg.to_radians();
    let y_tile = (n * (1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0) as i32;

    (x_tile, y_tile)
}

// Convert tile numbers to geographic coordinates (in degrees)
pub fn tile_to_geo(xtile: i32, ytile: i32, zoom: u32) -> (f64, f64) {
    let n = 2.0f64.powi(zoom as i32);
    let lon_deg = xtile as f64 / n * 360.0 - 180.0;
    
    // CORRECTED LATITUDE CALCULATION
    let lat_rad = (PI * (1.0 - 2.0 * ytile as f64  / n)).sinh().atan();
    
    (lon_deg, lat_rad.to_degrees())
}

pub fn lat_lon_to_tile_coords(lat: f32, lon: f32, zoom: i32) -> (i32, i32) {
    let x = ((lon + 180.0) / 360.0 * (2_i32.pow(zoom as u32) as f32)).floor() as i32;
    let y = ((1.0 - (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln() / std::f32::consts::PI) / 2.0 * (2_i32.pow(zoom as u32) as f32)).floor() as i32;
    (x, y)
}

pub fn tile_coords_to_lat_lon(x: i32, y: i32, zoom: i32) -> (f32, f32) {
    let n = 2_i32.pow(zoom as u32) as f32;
    let lon = x as f32 / n * 360.0 - 180.0;
    let lat_rad = (std::f32::consts::PI * (1.0 - 2.0 * y as f32 / n)).sinh().atan();
    let lat = lat_rad.to_degrees();
    (lat, lon)
}

pub fn world_mercator_to_lat_lon(
    x_offset: f64,
    y_offset: f64,
    reference: Coord, // Reference point in lat/lon (degrees)
) -> (f64, f64) {
    // Convert reference point to Web Mercator
    let (ref_x, ref_y) = lat_lon_to_world_mercator(reference.lat, reference.long);

    // Calculate meters per pixel (adjust for your tile setup)
    let meters_per_tile = 20037508.34 * 2.0 / (2.0_f64.powi(STARTING_ZOOM as i32)); // At zoom level N
    let scale = meters_per_tile / TILE_QUALITY as f64;

    // 1213.511890746124
    // Apply offsets with corrected scale
    let global_x = ref_x + (x_offset * scale).round();
    let global_y = ref_y + (y_offset * scale).round();
    info!("Global x: {}, Global y: {}", (x_offset * scale).round(), (y_offset * scale).round());
    // Inverse Mercator to convert back to lat/lon
    let lon = (global_x / 20037508.34) * 180.0;
    let lat = (global_y / 20037508.34 * 180.0).to_radians();
    let lat = 2.0 * lat.exp().atan() - std::f64::consts::FRAC_PI_2;
    let lat = lat.to_degrees();

    (lat, lon)
}

// Helper: Convert lat/lon (degrees) to global Mercator meters (EPSG:3857)
fn lat_lon_to_world_mercator(lat: f32, lon: f32) -> (f64, f64) {
    let lon_rad = lon.to_radians() as f64;
    let lat_rad = lat.to_radians() as f64;
    
    // Earth's circumference (meters) for Web Mercator (WGS84)
    const EARTH_RADIUS: f64 = 20037508.34;

    let x = lon_rad * EARTH_RADIUS / std::f64::consts::PI;
    
    // CORRECTED Y formula: ln(tan(π/4 + φ/2)) * radius
    let y = (std::f64::consts::FRAC_PI_4 + lat_rad / 2.0).tan().ln() * EARTH_RADIUS / std::f64::consts::PI;

    (x, y)
}