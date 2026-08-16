#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    pub base_x: f32,
    pub base_y: f32,
    pub strafe: f32,
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub alt_fire: bool,
    pub alt_fire_pressed: bool,
    pub alt_fire_released: bool,
    pub space_pressed: bool,
    pub space_released: bool,
    pub jump: bool,
    pub broom_pitch_up: bool,
    pub broom_pitch_down: bool,
    pub broom_boost: bool,
    pub broom_brake: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub actor: usize,
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub fov_degrees: f32,
    pub flash_fog: [f32; 4],
}
