use std::{
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{Error, Palette, Result, texture::rgba};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FireSpark {
    pub kind: u8,
    pub heat: u8,
    pub x: u8,
    pub y: u8,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
}

impl FireSpark {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            kind: bytes[0],
            heat: bytes[1],
            x: bytes[2],
            y: bytes[3],
            a: bytes[4],
            b: bytes[5],
            c: bytes[6],
            d: bytes[7],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FireTexture {
    pub spark_type: u8,
    pub render_heat: u8,
    pub rising: bool,
    pub fx_heat: u8,
    pub fx_size: u8,
    pub fx_aux_size: u8,
    pub fx_area: u8,
    pub fx_frequency: u8,
    pub fx_phase: u8,
    pub fx_horiz_speed: u8,
    pub fx_vert_speed: u8,
    pub draw_mode: u8,
    pub sparks_limit: usize,
    pub num_sparks: usize,
    pub sparks: Vec<FireSpark>,
}

impl Default for FireTexture {
    fn default() -> Self {
        Self {
            spark_type: 4,
            render_heat: 220,
            rising: false,
            fx_heat: 255,
            fx_size: 96,
            fx_aux_size: 0,
            fx_area: 24,
            fx_frequency: 16,
            fx_phase: 16,
            fx_horiz_speed: 130,
            fx_vert_speed: 142,
            draw_mode: 0,
            sparks_limit: 1024,
            num_sparks: 0,
            sparks: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FireAnimation {
    config: FireTexture,
    width: usize,
    height: usize,
    indices: Vec<u8>,
    render_table: [u8; 1024],
    accumulator: f32,
    prime_count: u8,
    prime_current: u8,
    min_frame_rate: f32,
    max_frame_rate: f32,
    global_phase: u8,
    frame: u32,
    star_status: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FireRng {
    pub table: [u8; 512],
    pub index: usize,
}

impl FireRng {
    pub const fn new(table: [u8; 512], index: usize) -> Self {
        Self {
            table,
            index: index & 0xfc,
        }
    }

    pub fn next_u32(&mut self) -> u32 {
        let source = (self.index + 0x80) & 0xfc;
        let value = u32::from_le_bytes(self.table[source..source + 4].try_into().unwrap());
        self.index = (self.index + 4) & 0xfc;
        let old = u32::from_le_bytes(self.table[self.index..self.index + 4].try_into().unwrap());
        self.table[self.index..self.index + 4].copy_from_slice(&(old ^ value).to_le_bytes());
        value
    }
}

static FIRE_RNG: OnceLock<Mutex<FireRng>> = OnceLock::new();

fn fire_rng() -> &'static Mutex<FireRng> {
    // Native consumes the current process-global Core appRand stream. Seeding
    // MSVCRT rand from current UNIX seconds approximates the shipped startup,
    // but cannot reproduce any appRand calls made before Fire initializes.
    // Callers may inject that exact observed state without changing Fire's
    // source-backed 512-byte transition.
    FIRE_RNG.get_or_init(|| {
        let mut table = [0; 512];
        let mut state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        for byte in &mut table {
            // Shipped Core appRand reaches MSVCRT rand(), seeded by
            // srand(time(NULL)) during Core initialization.
            *byte = msvcrt_rand(&mut state) as u8;
        }
        Mutex::new(FireRng::new(table, 0))
    })
}

fn msvcrt_rand(state: &mut u32) -> u16 {
    *state = state.wrapping_mul(214_013).wrapping_add(2_531_011);
    ((*state >> 16) & 0x7fff) as u16
}

impl FireTexture {
    pub fn animate(
        &self,
        width: u32,
        height: u32,
        prime_count: u8,
        min_frame_rate: f32,
        max_frame_rate: f32,
    ) -> Result<FireAnimation> {
        let mut animation =
            self.animate_unprimed(width, height, prime_count, min_frame_rate, max_frame_rate)?;
        animation.prime(&mut fire_rng().lock().unwrap());
        Ok(animation)
    }

    pub fn animate_with_rng(
        &self,
        width: u32,
        height: u32,
        prime_count: u8,
        min_frame_rate: f32,
        max_frame_rate: f32,
        rng: &mut FireRng,
    ) -> Result<FireAnimation> {
        let mut animation =
            self.animate_unprimed(width, height, prime_count, min_frame_rate, max_frame_rate)?;
        animation.prime(rng);
        Ok(animation)
    }

    fn animate_unprimed(
        &self,
        width: u32,
        height: u32,
        prime_count: u8,
        min_frame_rate: f32,
        max_frame_rate: f32,
    ) -> Result<FireAnimation> {
        if width == 0 || height == 0 || !width.is_power_of_two() || !height.is_power_of_two() {
            return Err(Error::InvalidFireDimensions { width, height });
        }
        let mut config = self.clone();
        config
            .sparks
            .resize(config.sparks_limit, FireSpark::default());
        config.num_sparks = config.num_sparks.min(config.sparks_limit);
        for (index, spark) in config.sparks[..config.num_sparks].iter().enumerate() {
            if u32::from(spark.x) >= width || u32::from(spark.y) >= height {
                return Err(Error::InvalidActiveFireSparkCoordinates {
                    index,
                    x: spark.x,
                    y: spark.y,
                    width,
                    height,
                });
            }
        }
        Ok(FireAnimation {
            config,
            width: width as usize,
            height: height as usize,
            indices: vec![0; width as usize * height as usize],
            render_table: fire_render_table(self.render_heat),
            accumulator: 0.0,
            prime_count: prime_count.max(48),
            prime_current: 0,
            min_frame_rate,
            max_frame_rate,
            global_phase: 0,
            frame: 0,
            star_status: true,
        })
    }
}

impl FireAnimation {
    pub fn tick(&mut self, delta_time: f32) -> bool {
        self.tick_with_rng(delta_time, &mut fire_rng().lock().unwrap())
    }

    pub fn tick_with_rng(&mut self, delta_time: f32, rng: &mut FireRng) -> bool {
        if !delta_time.is_finite() || delta_time == 0.0 {
            return false;
        }
        if self.max_frame_rate == 0.0 {
            return self.step(rng);
        }
        self.accumulator += delta_time;
        let maximum = frame_rate(self.max_frame_rate);
        if self.accumulator < 1.0 / maximum {
            return false;
        }
        let minimum_period = 1.0 / frame_rate(self.min_frame_rate);
        if self.accumulator < minimum_period {
            self.accumulator = 0.0;
        } else {
            self.accumulator -= minimum_period;
            self.accumulator = self.accumulator.min(minimum_period);
        }
        self.step(rng)
    }

    pub fn indices(&self) -> &[u8] {
        &self.indices
    }

    pub fn rgba(&self, palette: &Palette, masked: bool) -> Result<Vec<u8>> {
        rgba(&self.indices, palette, masked)
    }

    fn prime(&mut self, rng: &mut FireRng) {
        while self.prime_current < self.prime_count {
            self.prime_current += 1;
            let _ = self.step(rng);
        }
    }

    fn step(&mut self, rng: &mut FireRng) -> bool {
        if self.width <= 7 || self.height <= 7 {
            return false;
        }
        self.redraw_sparks(rng);
        filter_fire(
            &mut self.indices,
            self.width,
            self.height,
            &self.render_table,
            self.config.rising,
        );
        self.post_draw_sparks();
        true
    }

    fn pixel(&mut self, x: u8, y: u8, heat: u8) {
        self.indices[usize::from(x) + usize::from(y) * self.width] = heat;
    }

    fn append_with(&mut self, update: impl FnOnce(&mut FireSpark)) {
        if self.config.num_sparks < self.config.sparks_limit {
            let index = self.config.num_sparks;
            self.config.num_sparks += 1;
            update(&mut self.config.sparks[index]);
        }
    }

    fn remove(&mut self, index: usize) {
        self.config.num_sparks -= 1;
        self.config.sparks[index] = self.config.sparks[self.config.num_sparks];
    }

    fn move_xy(&self, spark: &mut FireSpark, dx: i8, dy: i8, rng: &mut FireRng) {
        move_axis(&mut spark.x, dx, self.width - 1, rng);
        move_axis(&mut spark.y, dy, self.height - 1, rng);
    }

    fn redraw_sparks(&mut self, rng: &mut FireRng) {
        self.global_phase = self.global_phase.wrapping_add(self.config.fx_frequency);
        self.frame = self.frame.wrapping_add(1);
        let mut i = 0;
        while i < self.config.num_sparks {
            let mut s = self.config.sparks[i];
            let mut deleted = false;
            match s.kind {
                0 => self.pixel(s.x, s.y, rng.next_u32() as u8),
                1 => {
                    let x = s.x.wrapping_add(
                        ((u16::from(s.a) * (rng.next_u32() as u8) as u16) >> 8) as u8,
                    ) & (self.width - 1) as u8;
                    let y = s.y.wrapping_add(
                        ((u16::from(s.b) * (rng.next_u32() as u8) as u16) >> 8) as u8,
                    ) & (self.height - 1) as u8;
                    self.pixel(x, y, s.heat);
                }
                2 => {
                    self.pixel(s.x, s.y, s.heat);
                    s.heat = s.heat.wrapping_add(s.d);
                }
                3 => {
                    let old = s.heat;
                    if s.c < old {
                        self.pixel(s.x, s.y, old);
                    }
                    s.heat = s.heat.wrapping_add(s.d);
                    if s.heat < s.d {
                        s.heat = rng.next_u32() as u8;
                    }
                }
                4..=8 => {
                    let threshold = if s.kind < 6 { 128 } else { 64 };
                    if self.config.num_sparks < self.config.sparks_limit
                        && (rng.next_u32() as u8) < threshold
                    {
                        let kind = if s.kind == 4 {
                            0x20
                        } else if s.kind == 5 {
                            0x21
                        } else {
                            0x22
                        };
                        let a = match s.kind {
                            4 => rng.next_u32() as u8,
                            5 | 6 => (rng.next_u32() as u8 & 0x7f).wrapping_sub(0x3f),
                            7 => (rng.next_u32() as u8 & 0x3f).wrapping_add(0x3f),
                            8 => (rng.next_u32() as u8 & 0x3f).wrapping_add(0x80),
                            _ => unreachable!(),
                        };
                        let random_b = (s.kind == 4).then(|| rng.next_u32() as u8);
                        self.append_with(|n| {
                            n.kind = kind;
                            n.heat = s.heat;
                            n.x = s.x;
                            n.y = s.y;
                            n.a = a;
                            match s.kind {
                                4 => {
                                    n.b = random_b.unwrap();
                                    n.c = s.c;
                                    n.d = s.d;
                                }
                                5 => {
                                    n.b = 0x81;
                                    n.d = 2;
                                }
                                6 => {
                                    n.b = 0;
                                    n.c = 0x32;
                                }
                                7 | 8 => {
                                    n.b = 0xe3;
                                    n.c = s.c;
                                }
                                _ => unreachable!(),
                            }
                        });
                    }
                }
                9 | 10 | 12 => {
                    let angle = s.a;
                    let vertical = sine(angle.wrapping_add(64));
                    if s.kind != 10 || angle.wrapping_add(64) < 128 {
                        let heat = s.heat.saturating_add(vertical);
                        let x = if s.kind == 12 {
                            s.x
                        } else {
                            s.x.wrapping_add(((u16::from(sine(angle)) * u16::from(s.b)) >> 8) as u8)
                        } & (self.width - 1) as u8;
                        let y = if s.kind == 12 {
                            s.y.wrapping_add(((u16::from(sine(angle)) * u16::from(s.b)) >> 8) as u8)
                        } else {
                            s.y
                        } & (self.height - 1) as u8;
                        self.pixel(x, y, heat);
                    }
                    s.a = s.a.wrapping_add(s.d);
                }
                11 => {
                    let x =
                        s.x.wrapping_add(((u16::from(sine(s.a)) * u16::from(s.heat)) >> 8) as u8)
                            & (self.width - 1) as u8;
                    let y =
                        s.y.wrapping_add(((u16::from(sine(s.b)) * u16::from(s.heat)) >> 8) as u8)
                            & (self.height - 1) as u8;
                    self.pixel(x, y, sine(s.a.wrapping_add(64)).saturating_add(32));
                    s.a = s.a.wrapping_add(s.c);
                    s.b = s.b.wrapping_add(s.d);
                }
                13 | 14 => {
                    if self.config.num_sparks < self.config.sparks_limit
                        && (rng.next_u32() as u8) < 64
                    {
                        self.append_with(|n| {
                            n.kind = if s.kind == 13 { 0x21 } else { 0x2a };
                            n.heat = s.heat;
                            n.x = s.x;
                            n.y = s.y;
                            n.a = s.a;
                            n.b = s.b;
                            n.d = s.d;
                        });
                    }
                }
                15 => {
                    if self.config.num_sparks < self.config.sparks_limit {
                        let x =
                            s.x.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.width - 1) as u8;
                        let y =
                            s.y.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.height - 1) as u8;
                        self.append_with(|n| {
                            n.kind = 0x27;
                            n.heat = s.heat;
                            n.x = x;
                            n.y = y;
                            n.a = 0;
                            n.b = s.a;
                            n.c = s.b;
                            n.d = s.d;
                        });
                        s.a = s.a.wrapping_add(s.c);
                    }
                    s.x = jitter(s.x, self.width - 1, rng);
                    s.y = jitter(s.y, self.height - 1, rng);
                }
                16 => {
                    if (rng.next_u32() as u8) < 20
                        && self.config.num_sparks < self.config.sparks_limit
                    {
                        let x =
                            s.x.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.width - 1) as u8;
                        let y =
                            s.y.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.height - 1) as u8;
                        let a = rng.next_u32() as u8;
                        let b = rng.next_u32() as u8;
                        self.append_with(|n| {
                            n.kind = 0x26;
                            n.heat = s.heat;
                            n.x = x;
                            n.y = y;
                            n.a = a;
                            n.b = b;
                            n.c = s.c;
                        });
                    }
                    s.x = optional_jitter(s.x, self.width - 1, rng);
                    s.y = optional_jitter(s.y, self.height - 1, rng);
                }
                17 => {
                    if self.config.num_sparks < self.config.sparks_limit
                        && (rng.next_u32() as u8) < 128
                    {
                        let x = s.x.wrapping_add(
                            ((u16::from(rng.next_u32() as u8) * u16::from(s.c)) >> 8) as u8,
                        ) & (self.width - 1) as u8;
                        let y = s.y.wrapping_add(
                            ((u16::from(rng.next_u32() as u8) * u16::from(s.c)) >> 8) as u8,
                        ) & (self.height - 1) as u8;
                        let a = (rng.next_u32() as u8).wrapping_sub(0x7f);
                        self.append_with(|n| {
                            n.kind = 0x23;
                            n.x = x;
                            n.y = y;
                            n.a = a;
                            n.b = 0x81;
                            n.c = 0xff;
                        });
                    }
                }
                18 | 19 => {
                    if self.config.num_sparks < self.config.sparks_limit {
                        let x =
                            s.x.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.width - 1) as u8;
                        let y =
                            s.y.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.height - 1) as u8;
                        let a = if s.kind == 18 {
                            (rng.next_u32() as u8).wrapping_sub(0x7f)
                        } else {
                            (rng.next_u32() as u8 & 0x1f).wrapping_sub(0x0f)
                        };
                        self.append_with(|n| {
                            n.kind = if s.kind == 18 { 0x23 } else { 0x24 };
                            n.x = x;
                            n.y = y;
                            n.a = a;
                            n.b = 0x81;
                            n.c = if s.kind == 18 { 0xff } else { 0 };
                        });
                    }
                    s.x = optional_jitter(s.x, self.width - 1, rng);
                    s.y = optional_jitter(s.y, self.height - 1, rng);
                }
                20 | 21 => {
                    if self.config.num_sparks < self.config.sparks_limit {
                        let x = s.x.wrapping_add(if s.kind == 20 {
                            rng.next_u32() as u8 & 0x1f
                        } else {
                            ((u16::from(rng.next_u32() as u8) * u16::from(s.c)) >> 8) as u8
                        }) & (self.width - 1) as u8;
                        let y = s.y.wrapping_add(if s.kind == 20 {
                            rng.next_u32() as u8 & 0x1f
                        } else {
                            ((u16::from(rng.next_u32() as u8) * u16::from(s.c)) >> 8) as u8
                        }) & (self.height - 1) as u8;
                        self.append_with(|n| {
                            n.kind = 0x25;
                            n.x = x;
                            n.y = y;
                            n.a = s.a;
                            n.b = s.b;
                            n.c = s.d;
                        });
                    }
                    if s.kind == 20 {
                        s.x = jitter(s.x, self.width - 1, rng);
                        s.y = jitter(s.y, self.height - 1, rng);
                    }
                }
                22 => self.pixel(s.x, s.y, s.b),
                23 | 24 => {
                    let fading = s.kind == 24;
                    self.flash(&mut s, fading, rng);
                }
                25 => {
                    if (rng.next_u32() as u8) >= s.d {
                        let angle = rng.next_u32() as u8;
                        let radius = s.c;
                        let dx = ((u16::from(sine(angle)) * u16::from(radius)) >> 8) as i16
                            - i16::from(radius >> 1);
                        let dy = ((u16::from(sine(angle.wrapping_add(64))) * u16::from(radius))
                            >> 8) as i16
                            - i16::from(radius >> 1);
                        draw_flash_ramp(
                            &mut self.indices,
                            self.width,
                            self.height,
                            s.x,
                            s.y,
                            dx,
                            dy,
                            s.heat,
                            s.heat >> 2,
                            rng,
                        );
                    }
                }
                26 | 28 => {
                    if self.config.num_sparks < self.config.sparks_limit {
                        self.append_with(|n| {
                            n.kind = if s.kind == 26 { 0x27 } else { 0x28 };
                            n.heat = s.heat;
                            n.x = s.x;
                            n.y = s.y;
                            if s.kind == 26 {
                                n.a = 0;
                                n.b = s.a;
                                n.c = s.b;
                                n.d = s.d;
                            } else {
                                n.a = s.a;
                                n.b = s.b;
                                n.c = s.c;
                                n.d = 2;
                            }
                        });
                    }
                    s.a = s.a.wrapping_add(if s.kind == 26 { s.c } else { s.d });
                }
                27 => {
                    if (rng.next_u32() as u8) < 20
                        && self.config.num_sparks < self.config.sparks_limit
                    {
                        let x =
                            s.x.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.width - 1) as u8;
                        let y =
                            s.y.wrapping_add(rng.next_u32() as u8 & 0x1f) & (self.height - 1) as u8;
                        let a = rng.next_u32() as u8;
                        let d = rng.next_u32() as u8;
                        self.append_with(|n| {
                            n.kind = 0x2b;
                            n.heat = s.heat;
                            n.x = x;
                            n.y = y;
                            n.a = a;
                            n.c = s.c;
                            n.d = d;
                        });
                    }
                    s.x = jitter(s.x, self.width - 1, rng);
                    s.y = jitter(s.y, self.height - 1, rng);
                }
                29 | 30 => {
                    let angle = if s.kind == 29 { s.a } else { s.b };
                    let x = if s.kind == 29 {
                        s.x.wrapping_add(((u16::from(sine(angle)) * u16::from(s.heat)) >> 8) as u8)
                    } else {
                        s.x
                    } & (self.width - 1) as u8;
                    let y = if s.kind == 30 {
                        s.y.wrapping_add(((u16::from(sine(angle)) * u16::from(s.heat)) >> 8) as u8)
                    } else {
                        s.y
                    } & (self.height - 1) as u8;
                    self.pixel(x, y, sine(angle.wrapping_add(64)).saturating_add(32));
                    if s.kind == 29 {
                        s.a = s.a.wrapping_add(s.c);
                    } else {
                        s.b = s.b.wrapping_add(s.d);
                    }
                }
                31 => {}
                32 | 33 | 34 | 37 | 38 | 39 | 40 | 41 | 42 | 43 => {
                    deleted = self.particle(&mut s, rng);
                }
                35 => {
                    s.c = s.c.wrapping_sub(3);
                    if s.c < 0xbf {
                        deleted = true;
                    } else {
                        self.pixel(s.x, s.y, s.c);
                        move_axis(&mut s.x, s.a as i8, self.width - 1, rng);
                        s.y = s.y.wrapping_sub(2) & (self.height - 1) as u8;
                    }
                }
                36 => {
                    s.c = s.c.wrapping_add(4);
                    if s.c > 0xf9 {
                        deleted = true;
                    } else {
                        self.pixel(s.x, s.y, s.c);
                        move_axis(&mut s.x, s.a as i8, self.width - 1, rng);
                        s.y = s.y.wrapping_sub(2) & (self.height - 1) as u8;
                    }
                }
                _ => unreachable!(),
            }
            if deleted {
                self.remove(i);
            } else {
                self.config.sparks[i] = s;
            }
            i += 1; // Native skips a swap-removed replacement until next tick.
        }
    }

    fn flash(&mut self, s: &mut FireSpark, fading: bool, rng: &mut FireRng) {
        if s.heat == 0 {
            return;
        }
        if s.c == 0 {
            if (rng.next_u32() as u8) >= s.d {
                s.c = (rng.next_u32() as u8).wrapping_add(1) & 5;
            }
        } else {
            s.c -= 1;
            draw_flash_ramp(
                &mut self.indices,
                self.width,
                self.height,
                s.x,
                s.y,
                s.a as i8 as i16,
                s.b as i8 as i16,
                s.heat,
                if fading { s.heat >> 3 } else { s.heat },
                rng,
            );
        }
    }

    fn particle(&mut self, s: &mut FireSpark, rng: &mut FireRng) -> bool {
        match s.kind {
            32 => {
                s.heat = s.heat.wrapping_sub(5);
                if s.heat >= 0xfb {
                    return true;
                }
            }
            33 => {
                let d = s.d;
                s.heat = s.heat.wrapping_sub(d);
                if s.heat <= d {
                    return true;
                }
            }
            34 => {
                s.c = s.c.wrapping_sub(1);
                if s.c == 0 {
                    return true;
                }
            }
            37 => {
                s.c = s.c.wrapping_add(4);
                if s.c > 0xf9 {
                    return true;
                }
            }
            38 | 39 | 40 | 43 => {
                s.c = s.c.wrapping_sub(1);
                if s.c == 0xff {
                    return true;
                }
            }
            41 => {
                s.heat = s.heat.wrapping_sub(s.c);
                if s.heat > 0xf9 {
                    return true;
                }
            }
            42 => {
                s.heat = s.heat.wrapping_sub(s.d);
                if s.heat < 0x33 {
                    return true;
                }
            }
            _ => {}
        }
        self.pixel(
            s.x,
            s.y,
            if matches!(s.kind, 35..=37) {
                s.c
            } else {
                s.heat
            },
        );
        if s.kind == 39 {
            let angle = s.b;
            let direction =
                u16::from_le_bytes([s.a, s.b]).wrapping_add(u16::from(s.d).wrapping_mul(0x10));
            [s.a, s.b] = direction.to_le_bytes();
            self.move_xy(
                s,
                centered_sine(angle.wrapping_add(64)),
                centered_sine(angle),
                rng,
            );
        } else if s.kind == 40 {
            let angle = s.a;
            s.a = s.a.wrapping_add(s.d);
            self.move_xy(
                s,
                sine(angle.wrapping_add(64)).wrapping_sub(128) as i8,
                s.b as i8,
                rng,
            );
        } else if s.kind == 43 {
            s.a = s.a.wrapping_add(7);
            let mut angle = s.a & 0x7f;
            if angle > 0x3f {
                angle = 0x7f - angle;
            }
            angle = angle.wrapping_add(s.d);
            self.move_xy(
                s,
                sine(angle).wrapping_sub(127) as i8,
                sine(angle.wrapping_add(64)).wrapping_sub(127) as i8,
                rng,
            );
        } else {
            self.move_xy(s, s.a as i8, s.b as i8, rng);
            if s.kind == 34 && (s.b as i8) < 122 {
                s.b = s.b.wrapping_add(3);
            }
            if s.kind == 42 && self.frame & 1 != 0 && (s.b as i8) < 124 {
                s.b = s.b.wrapping_add(3);
            }
        }
        false
    }

    fn post_draw_sparks(&mut self) {
        if !self.star_status {
            return;
        }
        let mut found = false;
        for s in &mut self.config.sparks[..self.config.num_sparks] {
            if s.kind == 22 {
                found = true;
                let index = usize::from(s.x) + usize::from(s.y) * self.width;
                s.b = self.indices[index];
                if s.b < 38 {
                    self.indices[index] = s.a;
                }
            }
        }
        self.star_status = found;
    }
}

fn fire_render_table(render_heat: u8) -> [u8; 1024] {
    let mut table = [0; 1024];
    let loss = i32::from(255 - render_heat);
    for (i, value) in table.iter_mut().enumerate() {
        let numerator = i as i32 * 4 + 16 - loss;
        let quotient = numerator.div_euclid(16);
        let remainder = numerator.rem_euclid(16);
        let rounded = quotient + i32::from(remainder > 8 || (remainder == 8 && quotient & 1 != 0));
        *value = rounded.clamp(0, 255) as u8;
    }
    table
}

fn filter_fire(pixels: &mut [u8], width: usize, height: usize, table: &[u8; 1024], rising: bool) {
    let source = pixels.to_vec();
    for y in 0..height {
        let y0 = (y + usize::from(rising)) & (height - 1);
        let y1 = (y0 + 1) & (height - 1);
        for x in 0..width {
            let sum = usize::from(source[x + y0 * width])
                + usize::from(source[((x + 1) & (width - 1)) + y0 * width])
                + usize::from(source[((x + width - 1) & (width - 1)) + y1 * width])
                + usize::from(source[x + y1 * width]);
            pixels[x + y * width] = table[sum];
        }
    }
}

fn frame_rate(rate: f32) -> f32 {
    if rate >= 0.1 { rate.min(100.0) } else { 0.1 }
}

fn sine(angle: u8) -> u8 {
    static TABLE: OnceLock<[u8; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0; 256];
        for (angle, value) in table.iter_mut().enumerate() {
            *value = ((angle as f64 * 0.00390625 * 6.2831855).sin() * 127.5 + 127.44999694824219)
                .round_ties_even()
                .clamp(0.0, 255.0) as u8;
        }
        table
    })[usize::from(angle)]
}

fn centered_sine(angle: u8) -> i8 {
    sine(angle).wrapping_add(0x80) as i8
}

fn move_axis(value: &mut u8, speed: i8, mask: usize, rng: &mut FireRng) {
    let threshold = speed.unsigned_abs();
    if (rng.next_u32() as u8 & 0x7f) < threshold {
        *value = if speed < 0 {
            value.wrapping_sub(1)
        } else {
            value.wrapping_add(1)
        } & mask as u8;
    }
}

fn jitter(value: u8, mask: usize, rng: &mut FireRng) -> u8 {
    value.wrapping_add((rng.next_u32() as u8 & 7).wrapping_sub(rng.next_u32() as u8 & 7))
        & mask as u8
}

fn optional_jitter(value: u8, mask: usize, rng: &mut FireRng) -> u8 {
    if rng.next_u32() as u8 & 1 == 0 {
        value
    } else {
        value.wrapping_add((rng.next_u32() as u8 & 0x0f).wrapping_sub(7)) & mask as u8
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_flash_ramp(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    mut x: u8,
    mut y: u8,
    dx: i16,
    dy: i16,
    mut start: u8,
    mut end: u8,
    rng: &mut FireRng,
) {
    let mut packed_dx = if dx < 1 {
        (-dx as u8) | 1
    } else {
        (dx as u8) & 0xfe
    };
    let mut packed_dy = if dy < 1 {
        (-dy as u8) | 1
    } else {
        (dy as u8) & 0xfe
    };
    let reverse = if packed_dy & 1 != 0 {
        if 2 * u32::from(packed_dy) >= u32::from(packed_dx) {
            true
        } else {
            packed_dx & 1 != 0
        }
    } else if packed_dx & 1 != 0 {
        2 * u32::from(packed_dy) < u32::from(packed_dx)
    } else {
        false
    };
    if reverse {
        x = x.wrapping_add(if packed_dx & 1 == 0 {
            packed_dx
        } else {
            packed_dx.wrapping_neg()
        });
        y = y.wrapping_add(if packed_dy & 1 == 0 {
            packed_dy
        } else {
            packed_dy.wrapping_neg()
        });
        packed_dx ^= 1;
        packed_dy ^= 1;
        std::mem::swap(&mut start, &mut end);
    }
    let count = usize::from(packed_dx.max(packed_dy) | 1);
    let random = (0..count).map(|_| rng.next_u32() as u8).collect::<Vec<_>>();
    let random_sum = random.iter().map(|&value| i32::from(value)).sum::<i32>();
    let x_step: i8 = if packed_dx & 1 == 0 { 1 } else { -1 };
    let y_step: i8 = if packed_dy & 1 == 0 { 1 } else { -1 };
    let signed_dx = i32::from(packed_dx) * i32::from(x_step);
    let signed_dy = i32::from(packed_dy) * i32::from(y_step);
    let mut heat = i32::from(start) << 23;
    let heat_step = (i32::from(end) - i32::from(start)) * 0x80_0000 / count as i32;
    if packed_dx < packed_dy {
        let correction = (signed_dx * 64 - random_sum) / count as i32;
        let mut minor = i32::from(x) << 6;
        for &noise in random.iter().take(usize::from(packed_dy)) {
            minor += i32::from(noise) + correction;
            let index =
                (((minor >> 6) as usize) & (width - 1)) + (usize::from(y) & (height - 1)) * width;
            y = y.wrapping_add_signed(y_step);
            heat += heat_step;
            pixels[index] = (heat >> 23) as u8;
        }
    } else {
        let correction = (signed_dy * 64 - random_sum) / count as i32;
        let mut minor = i32::from(y) << 6;
        for &noise in random.iter().take(usize::from(packed_dx)) {
            minor += i32::from(noise) + correction;
            let index =
                (usize::from(x) & (width - 1)) + (((minor >> 6) as usize) & (height - 1)) * width;
            x = x.wrapping_add_signed(x_step);
            heat += heat_step;
            pixels[index] = (heat >> 23) as u8;
        }
    }
}

#[cfg(test)]
fn draw_spark_line(
    mut x: i32,
    mut y: i32,
    end_x: i32,
    end_y: i32,
    mut visit: impl FnMut(i32, i32),
) {
    let dx = (end_x - x).abs();
    let dy = (end_y - y).abs();
    let sx = (end_x - x).signum();
    let sy = (end_y - y).signum();
    let mut error = dx - dy;
    while (x, y) != (end_x, end_y) {
        visit(x, y);
        let twice = error * 2;
        if twice > -dy {
            error -= dy;
            x += sx;
        }
        if twice < dx {
            error += dx;
            y += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_properties_use_native_constructor_defaults() {
        let fire = FireTexture::default();
        assert_eq!(
            [
                fire.spark_type,
                fire.render_heat,
                fire.fx_heat,
                fire.fx_size,
                fire.fx_aux_size,
                fire.fx_area,
                fire.fx_frequency,
                fire.fx_phase,
                fire.fx_horiz_speed,
                fire.fx_vert_speed,
                fire.draw_mode
            ],
            [4, 220, 255, 96, 0, 24, 16, 16, 130, 142, 0]
        );
        assert_eq!(fire.sparks_limit, 1024);
    }

    #[test]
    fn render_table_and_wrapped_filters_are_exact() {
        let table = fire_render_table(247);
        assert_eq!((table[0], table[2], table[3], table[1023]), (0, 1, 1, 255));
        let input = (0..64).collect::<Vec<u8>>();
        let mut flat = input.clone();
        filter_fire(&mut flat, 8, 8, &table, false);
        let mut rising = input;
        filter_fire(&mut rising, 8, 8, &table, true);
        assert_eq!((flat[0], flat[63]), (6, 34));
        assert_eq!((rising[0], rising[63]), (14, 10));
    }

    #[test]
    fn rng_wraps_and_xors_the_new_slot() {
        let mut seed = 1;
        assert_eq!(
            [
                msvcrt_rand(&mut seed),
                msvcrt_rand(&mut seed),
                msvcrt_rand(&mut seed)
            ],
            [41, 18_467, 6_334]
        );
        let mut bytes = [0; 512];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = i as u8;
        }
        let mut rng = FireRng::new(bytes, 0xfc);
        let expected = u32::from_le_bytes([0x7c, 0x7d, 0x7e, 0x7f]);
        assert_eq!(rng.next_u32(), expected);
        assert_eq!(rng.index, 0);
        assert_eq!(&rng.table[0..4], &(0x03020100_u32 ^ expected).to_le_bytes());
    }

    #[test]
    fn bresenham_excludes_endpoint_and_star_boundary_is_strict() {
        let mut points = Vec::new();
        draw_spark_line(1, 1, 4, 1, |x, y| points.push((x, y)));
        assert_eq!(points, [(1, 1), (2, 1), (3, 1)]);
        let config = FireTexture {
            spark_type: 0,
            render_heat: 255,
            rising: false,
            fx_heat: 0,
            fx_size: 0,
            fx_aux_size: 0,
            fx_area: 0,
            fx_frequency: 0,
            fx_phase: 0,
            fx_horiz_speed: 0,
            fx_vert_speed: 0,
            draw_mode: 0,
            sparks_limit: 2,
            num_sparks: 1,
            sparks: vec![FireSpark {
                kind: 22,
                x: 1,
                y: 1,
                a: 99,
                ..Default::default()
            }],
        };
        let mut animation = config.animate(8, 8, 0, 0.0, 0.0).unwrap();
        animation.indices[9] = 37;
        animation.post_draw_sparks();
        assert_eq!(animation.indices[9], 99);
        animation.indices[9] = 38;
        animation.post_draw_sparks();
        assert_eq!(animation.indices[9], 38);
    }

    #[test]
    fn flash_ramp_diagonal_uses_precomputed_noise_and_fixed_point_heat() {
        let mut pixels = vec![0; 64];
        let mut rng = FireRng::new([0; 512], 0);
        draw_flash_ramp(&mut pixels, 8, 8, 1, 1, 4, 2, 100, 40, &mut rng);
        assert_eq!(
            (pixels[9], pixels[10], pixels[19], pixels[20], rng.index),
            (88, 76, 64, 52, 20)
        );
        let mut pixels = vec![0; 256 * 256];
        let mut rng = FireRng::new([0; 512], 0);
        draw_flash_ramp(
            &mut pixels,
            256,
            256,
            250,
            250,
            -201,
            -129,
            200,
            100,
            &mut rng,
        );
        assert_ne!(pixels[49 + 121 * 256], 0);
        assert_ne!(pixels[248 + 246 * 256], 0);
        assert_eq!(rng.index, 36);

        let mut pixels = vec![0; 256 * 256];
        let mut rng = FireRng::new([0; 512], 0);
        draw_flash_ramp(
            &mut pixels,
            256,
            256,
            250,
            250,
            -201,
            -65,
            200,
            100,
            &mut rng,
        );
        assert_ne!(pixels[49 + 185 * 256], 0, "reversed first coordinate");
        assert_ne!(pixels[248 + 247 * 256], 0, "reversed last coordinate");
        assert_eq!(rng.index, 36);
    }

    #[test]
    fn manhattan_delete_and_swap_order_match_native() {
        let mut sparks = vec![
            FireSpark {
                x: 1,
                y: 1,
                ..Default::default()
            },
            FireSpark {
                x: 2,
                y: 2,
                ..Default::default()
            },
            FireSpark {
                x: 7,
                y: 7,
                ..Default::default()
            },
        ];
        let mut i = 0;
        while i < sparks.len() {
            let s = sparks[i];
            if i16::from(s.x).abs_diff(1) + i16::from(s.y).abs_diff(1) <= 2 {
                sparks.swap_remove(i);
            }
            i += 1;
        }
        assert_eq!(
            sparks,
            [FireSpark {
                x: 7,
                y: 7,
                ..Default::default()
            }]
        );
    }

    #[test]
    fn append_reuses_backing_bytes_and_delete_skips_the_swapped_spark() {
        let config = FireTexture {
            sparks_limit: 3,
            num_sparks: 1,
            sparks: vec![
                spark([0x20, 1, 4, 4, 0, 0, 0, 0]),
                spark([0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44]),
                spark([0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc]),
            ],
            ..FireTexture::default()
        };
        let mut animation = config.animate_unprimed(8, 8, 48, 0.0, 0.0).unwrap();
        animation.append_with(|spark| spark.kind = 0x2b);
        assert_eq!(
            animation.config.sparks[1],
            spark([0x2b, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44])
        );

        animation.config.sparks[0] = spark([0x20, 1, 4, 4, 0, 0, 0, 0]);
        animation.config.sparks[1] = spark([2, 128, 4, 4, 0, 0, 0, 0]);
        animation.config.num_sparks = 2;
        let mut rng = FireRng::new([0; 512], 0);
        animation.redraw_sparks(&mut rng);
        assert_eq!(animation.config.num_sparks, 1);
        assert_eq!(
            animation.config.sparks[0],
            spark([2, 128, 4, 4, 0, 0, 0, 0])
        );
        assert_eq!(animation.indices, vec![0; 64], "swapped spark is skipped");
        assert_eq!(animation.config.sparks.len(), 3, "backing is not shrunk");
    }

    fn spark(bytes: [u8; 8]) -> FireSpark {
        FireSpark::from_bytes(&bytes)
    }

    fn redraw_zero_rng(kind: u8, c: u8) -> (FireAnimation, FireRng) {
        let config = FireTexture {
            spark_type: kind,
            render_heat: 255,
            rising: false,
            fx_frequency: 1,
            sparks_limit: 4,
            num_sparks: 1,
            sparks: vec![
                spark([kind, 128, 4, 4, 1, 1, c, 1]),
                spark([0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44]),
                spark([0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc]),
                spark([0xdd, 0xee, 0xff, 0x10, 0x20, 0x30, 0x40, 0x50]),
            ],
            ..FireTexture::default()
        };
        let mut animation = config.animate_unprimed(8, 8, 48, 0.0, 0.0).unwrap();
        let mut rng = FireRng::new([0; 512], 0);
        animation.redraw_sparks(&mut rng);
        (animation, rng)
    }

    fn assert_redraw_state(
        animation: &FireAnimation,
        rng: &FireRng,
        active: usize,
        backing: [[u8; 8]; 4],
        pixel: Option<(usize, u8)>,
        rng_index: usize,
    ) {
        assert_eq!(animation.config.num_sparks, active);
        assert_eq!(
            animation.config.sparks,
            backing.map(spark),
            "complete serialized backing slots"
        );
        let mut pixels = vec![0; 64];
        if let Some((index, heat)) = pixel {
            pixels[index] = heat;
        }
        assert_eq!(animation.indices, pixels, "complete 8x8 output");
        assert_eq!(rng.index, rng_index);
        assert_eq!(rng.table, [0; 512]);
        assert_eq!(
            (
                animation.global_phase,
                animation.frame,
                animation.star_status,
                animation.accumulator,
                animation.prime_current,
            ),
            (1, 1, true, 0.0, 0)
        );
    }

    #[test]
    fn corrected_public_spawn_cases_match_native_full_state() {
        let inactive_2 = [0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc];
        let inactive_3 = [0xdd, 0xee, 0xff, 0x10, 0x20, 0x30, 0x40, 0x50];

        let (animation, rng) = redraw_zero_rng(0x10, 1);
        assert_redraw_state(
            &animation,
            &rng,
            2,
            [
                [0x10, 128, 4, 4, 1, 1, 1, 1],
                [0x26, 128, 4, 4, 0, 0, 0, 0x44],
                inactive_2,
                inactive_3,
            ],
            Some((36, 128)),
            36,
        );

        let (animation, rng) = redraw_zero_rng(0x12, 1);
        assert_redraw_state(
            &animation,
            &rng,
            2,
            [
                [0x12, 128, 4, 4, 1, 1, 1, 1],
                [0x23, 0xbb, 3, 2, 0x81, 0x81, 0xfc, 0x44],
                inactive_2,
                inactive_3,
            ],
            Some((36, 0xfc)),
            24,
        );

        let (animation, rng) = redraw_zero_rng(0x13, 1);
        assert_redraw_state(
            &animation,
            &rng,
            2,
            [
                [0x13, 128, 4, 4, 1, 1, 1, 1],
                [0x24, 0xbb, 3, 2, 0xf1, 0x81, 4, 0x44],
                inactive_2,
                inactive_3,
            ],
            Some((36, 4)),
            24,
        );

        let (animation, rng) = redraw_zero_rng(0x1a, 1);
        assert_redraw_state(
            &animation,
            &rng,
            2,
            [
                [0x1a, 128, 4, 4, 2, 1, 1, 1],
                [0x27, 128, 5, 5, 16, 1, 0, 1],
                inactive_2,
                inactive_3,
            ],
            Some((36, 128)),
            8,
        );
    }

    #[test]
    fn corrected_internal_heat_and_centered_sine_cases_match_native_full_state() {
        let inactive_1 = [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44];
        let inactive_2 = [0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc];
        let inactive_3 = [0xdd, 0xee, 0xff, 0x10, 0x20, 0x30, 0x40, 0x50];

        let (animation, rng) = redraw_zero_rng(0x06, 1);
        assert_redraw_state(
            &animation,
            &rng,
            2,
            [
                [0x06, 128, 4, 4, 1, 1, 1, 1],
                [0x22, 128, 3, 4, 0xc1, 3, 0x31, 0x44],
                inactive_2,
                inactive_3,
            ],
            Some((36, 128)),
            16,
        );

        let (animation, rng) = redraw_zero_rng(0x27, 1);
        assert_redraw_state(
            &animation,
            &rng,
            1,
            [
                [0x27, 128, 5, 5, 17, 1, 0, 1],
                inactive_1,
                inactive_2,
                inactive_3,
            ],
            Some((36, 128)),
            8,
        );
    }

    fn shipped_case_rng() -> FireRng {
        let mut table = [0; 512];
        for word in 0..128 {
            table[word * 4..word * 4 + 4].copy_from_slice(&[
                ((word + 96) & 127) as u8,
                word as u8,
                word as u8 ^ 0xa5,
                0x5a,
            ]);
        }
        FireRng::new(table, 0)
    }

    fn shipped_case_rng_after(calls: usize) -> FireRng {
        let initial = shipped_case_rng();
        let mut expected = initial.clone();
        for call in 0..calls {
            let source = (32 + call) * 4;
            let destination = (1 + call) * 4;
            for byte in 0..4 {
                expected.table[destination + byte] ^= initial.table[source + byte];
            }
        }
        expected.index = calls * 4;
        expected
    }

    #[test]
    fn shipped_redraw_cases_match_independently_derived_native_literals() {
        struct Row {
            kind: u8,
            initial: [u8; 8],
            active: usize,
            first: [u8; 8],
            second: [u8; 8],
            pixel: (usize, u8),
            calls: usize,
        }

        let rows = [
            Row {
                kind: 0x00,
                initial: [0x00, 128, 4, 4, 1, 1, 1, 1],
                active: 1,
                first: [0x00, 128, 4, 4, 1, 1, 1, 1],
                second: [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44],
                pixel: (36, 0),
                calls: 1,
            },
            Row {
                kind: 0x01,
                initial: [0x01, 128, 4, 4, 1, 1, 1, 1],
                active: 1,
                first: [0x01, 128, 4, 4, 1, 1, 1, 1],
                second: [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44],
                pixel: (36, 128),
                calls: 2,
            },
            Row {
                kind: 0x03,
                initial: [0x03, 255, 4, 4, 1, 1, 0, 1],
                active: 1,
                first: [0x03, 0, 4, 4, 1, 1, 0, 1],
                second: [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44],
                pixel: (36, 255),
                calls: 1,
            },
            Row {
                kind: 0x0c,
                initial: [0x0c, 128, 4, 4, 1, 1, 1, 1],
                active: 1,
                first: [0x0c, 128, 4, 4, 2, 1, 1, 1],
                second: [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44],
                pixel: (36, 255),
                calls: 0,
            },
            Row {
                kind: 0x1b,
                initial: [0x1b, 128, 4, 4, 1, 1, 1, 1],
                active: 2,
                first: [0x1b, 128, 3, 3, 1, 1, 1, 1],
                second: [0x2b, 128, 6, 7, 10, 0x22, 0, 4],
                pixel: (53, 128),
                calls: 11,
            },
            Row {
                kind: 0x2b,
                initial: [0x2b, 128, 4, 4, 1, 1, 1, 1],
                active: 1,
                first: [0x2b, 128, 5, 5, 8, 1, 0, 1],
                second: [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44],
                pixel: (36, 128),
                calls: 2,
            },
        ];
        let inactive_2 = [0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc];
        let inactive_3 = [0xdd, 0xee, 0xff, 0x10, 0x20, 0x30, 0x40, 0x50];
        let baseline = (0..64)
            .map(|index| ((37 * index + 11) & 255) as u8)
            .collect::<Vec<_>>();

        for row in rows {
            let config = FireTexture {
                spark_type: row.kind,
                render_heat: 255,
                fx_frequency: 1,
                sparks_limit: 4,
                num_sparks: 1,
                sparks: [
                    row.initial,
                    [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44],
                    inactive_2,
                    inactive_3,
                ]
                .map(spark)
                .to_vec(),
                ..FireTexture::default()
            };
            let mut animation = config.animate_unprimed(8, 8, 48, 0.0, 0.0).unwrap();
            animation.indices.clone_from(&baseline);
            let mut rng = shipped_case_rng();
            animation.redraw_sparks(&mut rng);

            assert_eq!(
                animation.config.num_sparks, row.active,
                "case {:#04x}",
                row.kind
            );
            assert_eq!(
                animation.config.sparks,
                [row.first, row.second, inactive_2, inactive_3].map(spark),
                "case {:#04x} backing",
                row.kind
            );
            let mut pixels = baseline.clone();
            pixels[row.pixel.0] = row.pixel.1;
            assert_eq!(animation.indices, pixels, "case {:#04x} pixels", row.kind);
            assert_eq!(
                rng,
                shipped_case_rng_after(row.calls),
                "case {:#04x} RNG",
                row.kind
            );
            assert_eq!(
                (
                    animation.global_phase,
                    animation.frame,
                    animation.star_status,
                    animation.accumulator,
                    animation.prime_current,
                ),
                (1, 1, true, 0.0, 0),
                "case {:#04x} globals",
                row.kind
            );
        }
    }

    #[test]
    fn prime_is_at_least_48_and_scheduler_ticks_once_per_update() {
        let config = FireTexture {
            spark_type: 0,
            render_heat: 255,
            rising: false,
            fx_heat: 0,
            fx_size: 0,
            fx_aux_size: 0,
            fx_area: 0,
            fx_frequency: 0,
            fx_phase: 0,
            fx_horiz_speed: 0,
            fx_vert_speed: 0,
            draw_mode: 0,
            sparks_limit: 0,
            num_sparks: 0,
            sparks: vec![],
        };
        for (requested, expected) in [(0, 48), (1, 48), (47, 48), (48, 48), (49, 49)] {
            let mut rng = FireRng::new([0; 512], 0);
            let mut a = config
                .animate_with_rng(8, 8, requested, 0.0, 0.0, &mut rng)
                .unwrap();
            assert!(a.tick_with_rng(1.0, &mut rng));
            assert_eq!(a.prime_current, expected);
            assert_eq!(a.frame, u32::from(expected) + 1);
        }
    }

    #[test]
    fn tiny_power_of_two_textures_prime_without_stepping() {
        let config = FireTexture {
            sparks_limit: 4,
            num_sparks: 1,
            sparks: vec![spark([0, 128, 0, 0, 0, 0, 0, 0])],
            ..FireTexture::default()
        };
        for (width, height) in [(1, 8), (8, 4)] {
            let mut rng = FireRng::new([0; 512], 0);
            let mut animation = config
                .animate_with_rng(width, height, 0, 0.0, 0.0, &mut rng)
                .unwrap();
            assert_eq!(animation.prime_current, 48);
            assert_eq!((animation.frame, animation.global_phase), (0, 0));
            assert_eq!(rng, FireRng::new([0; 512], 0));
            assert_eq!(animation.indices, vec![0; (width * height) as usize]);
            assert!(!animation.tick_with_rng(1.0, &mut rng));
        }
    }

    #[test]
    fn active_spark_coordinates_are_validated_but_inactive_backing_is_opaque() {
        let config = FireTexture {
            sparks_limit: 2,
            num_sparks: 1,
            sparks: vec![spark([2, 128, 8, 7, 0, 0, 0, 0]), spark([0xff; 8])],
            ..FireTexture::default()
        };
        let mut rng = FireRng::new([0; 512], 0);
        assert!(matches!(
            config.animate_with_rng(8, 8, 0, 0.0, 0.0, &mut rng),
            Err(Error::InvalidActiveFireSparkCoordinates {
                index: 0,
                x: 8,
                y: 7,
                width: 8,
                height: 8,
            })
        ));

        let inactive = FireTexture {
            num_sparks: 0,
            ..config
        };
        assert!(
            inactive
                .animate_with_rng(8, 8, 0, 0.0, 0.0, &mut rng)
                .is_ok()
        );
    }
}
