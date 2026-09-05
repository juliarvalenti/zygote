//! Two-layer simulation: permanent hyphal structure grown by wandering,
//! branching tips; animated nutrient flow traced by agents that follow the
//! structure. Ported from a canvas sketch; deterministic (seeded RNG, fixed
//! step rate on the transport clock) so scrubbing and pausing behave.

use zygote_render::prelude::*;

pub const COLS: usize = 480;
pub const ROWS: usize = 270;

#[derive(NodeParams, Clone, Debug)]
pub struct Params {
    /// Simulation steps per second of transport time
    #[param(default = 30.0, min = 1.0, max = 120.0)]
    pub sim_rate: f32,
    /// Tip growth speed multiplier
    #[param(default = 1.0, min = 0.0, max = 3.0)]
    pub growth: f32,
    /// Probability per step that a tip branches
    #[param(default = 0.015, min = 0.0, max = 0.06)]
    pub branching: f32,
    /// How much tips wander
    #[param(default = 0.1, min = 0.0, max = 0.5)]
    pub wander: f32,
    /// Nutrient agents on the network
    #[param(default = 900, min = 0, max = 2000)]
    pub agents: i32,
    /// Agent speed multiplier
    #[param(default = 1.0, min = 0.0, max = 3.0)]
    pub flow: f32,
    /// Agent sensor angle (radians)
    #[param(default = 0.6, min = 0.1, max = 1.5)]
    pub sensor_angle: f32,
    /// Per-step retention of the flow trail
    #[param(default = 0.99, min = 0.9, max = 1.0)]
    pub trail_decay: f32,
    /// Brightness of the animated flow
    #[param(default = 0.2, min = 0.0, max = 1.0)]
    pub flow_gain: f32,
    /// Brightness of the permanent structure
    #[param(default = 0.08, min = 0.0, max = 0.2)]
    pub structure_gain: f32,
    /// Ink color
    #[param(default = "#5cc7d2")]
    pub ink: [f32; 4],
    /// Background color
    #[param(default = "#0c0e11")]
    pub background: [f32; 4],
    /// Colonies seeded on reset
    #[param(default = 24, min = 1, max = 60)]
    pub colonies: i32,
    /// Flip on to wipe the network and regrow it
    #[param(default = false)]
    pub reset: bool,
}

/// Tiny deterministic RNG (xorshift32).
#[derive(Clone)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32) / (u32::MAX as f32)
    }
}

#[derive(Clone)]
struct Tip {
    x: f32,
    y: f32,
    angle: f32,
    ci: u8,
    age: u32,
    generation: u32,
    max_age: u32,
    speed: f32,
}

#[derive(Clone)]
struct Agent {
    x: f32,
    y: f32,
    angle: f32,
    ci: u8,
    speed: f32,
}

pub struct Mycelium {
    structure: Vec<f32>,
    trail: Vec<f32>,
    color_idx: Vec<u8>,
    tips: Vec<Tip>,
    agents: Vec<Agent>,
    rng: Rng,
    /// Fractional simulation steps carried between frames.
    step_debt: f32,
    reset_latch: bool,
}

const TIP_SPEED: f32 = 0.25;
const MAX_TIPS: usize = 1200;
const SENSOR_DIST: f32 = 6.0;
const BRANCH_ANGLE_MIN: f32 = 0.4;
const BRANCH_ANGLE_MAX: f32 = 1.1;

impl Mycelium {
    fn idx(x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= COLS as i32 || y >= ROWS as i32 {
            None
        } else {
            Some(y as usize * COLS + x as usize)
        }
    }

    fn deposit(&mut self, x: f32, y: f32, ci: u8, generation: u32) {
        let (gx, gy) = (x.floor() as i32, y.floor() as i32);
        let strength = 0.5 / (1.0 + generation as f32 * 0.25);
        let radius: i32 = if generation < 2 { 1 } else { 0 };
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if let Some(i) = Self::idx(gx + dx, gy + dy) {
                    let s = if dx == 0 && dy == 0 {
                        strength
                    } else {
                        strength * 0.4
                    };
                    self.structure[i] = (self.structure[i] + s).min(1.0);
                    self.color_idx[i] = ci;
                }
            }
        }
    }

    fn occupied(&self, x: f32, y: f32) -> bool {
        match Self::idx(x.floor() as i32, y.floor() as i32) {
            Some(i) => self.structure[i] > 0.2,
            None => true,
        }
    }

    fn seed_colony(&mut self, cx: f32, cy: f32, ci: u8, n: usize) {
        for i in 0..n {
            let angle = std::f32::consts::TAU * i as f32 / n as f32 + (self.rng.next() - 0.5) * 0.5;
            let max_age = 1500 + (self.rng.next() * 3000.0) as u32;
            let speed = TIP_SPEED * (0.7 + self.rng.next() * 0.6);
            self.tips.push(Tip {
                x: cx + angle.cos() * 2.0,
                y: cy + angle.sin() * 2.0,
                angle,
                ci,
                age: 0,
                generation: 0,
                max_age,
                speed,
            });
        }
        self.deposit(cx, cy, ci, 0);
    }

    fn grow_step(&mut self, p: &Params) {
        let mut new_tips: Vec<Tip> = Vec::new();
        let mut i = self.tips.len();
        while i > 0 {
            i -= 1;
            let mut t = self.tips[i].clone();
            t.age += 1;
            t.angle += (self.rng.next() - 0.5) * p.wander * 2.0;
            let speed = t.speed * p.growth;
            let nx = t.x + t.angle.cos() * speed;
            let ny = t.y + t.angle.sin() * speed;
            if nx < 0.0 || nx >= COLS as f32 || ny < 0.0 || ny >= ROWS as f32 || t.age > t.max_age {
                self.tips.swap_remove(i);
                continue;
            }
            // A tip stops when it runs into existing hypha, but only when it is
            // about to enter a new cell; its own freshly deposited cell does not count.
            let entering_new_cell = (nx.floor(), ny.floor()) != (t.x.floor(), t.y.floor());
            if t.age > 50 && entering_new_cell && self.occupied(nx, ny) {
                self.deposit(nx, ny, t.ci, t.generation);
                self.tips.swap_remove(i);
                continue;
            }
            self.deposit(t.x, t.y, t.ci, t.generation);
            t.x = nx;
            t.y = ny;
            if t.age > 300 && t.age.is_multiple_of(150) && self.rng.next() < 0.12 {
                let (x, y, ci) = (t.x, t.y, t.ci);
                self.tips[i] = t.clone();
                self.seed_colony(x, y, ci, 2);
            }
            if self.tips.len() + new_tips.len() < MAX_TIPS
                && self.rng.next() < p.branching
                && t.age > 10
            {
                let b_angle =
                    BRANCH_ANGLE_MIN + self.rng.next() * (BRANCH_ANGLE_MAX - BRANCH_ANGLE_MIN);
                let sign = if self.rng.next() < 0.5 { -1.0 } else { 1.0 };
                let max_age = 600 + (self.rng.next() * 1500.0) as u32;
                let speed = t.speed * (0.75 + self.rng.next() * 0.25);
                new_tips.push(Tip {
                    x: t.x,
                    y: t.y,
                    angle: t.angle + sign * b_angle,
                    ci: t.ci,
                    age: 0,
                    generation: t.generation + 1,
                    max_age,
                    speed,
                });
            }
            if i < self.tips.len() {
                self.tips[i] = t;
            }
        }
        for tip in new_tips {
            if self.tips.len() < MAX_TIPS {
                self.tips.push(tip);
            }
        }
    }

    fn spawn_agent(&mut self) {
        for _ in 0..20 {
            let x = self.rng.next() * COLS as f32;
            let y = self.rng.next() * ROWS as f32;
            if let Some(i) = Self::idx(x as i32, y as i32)
                && self.structure[i] > 0.1
            {
                let angle = self.rng.next() * std::f32::consts::TAU;
                let speed = 0.08 + self.rng.next() * 0.1;
                self.agents.push(Agent {
                    x,
                    y,
                    angle,
                    ci: self.color_idx[i],
                    speed,
                });
                return;
            }
        }
    }

    fn sense(&self, ax: f32, ay: f32, angle: f32) -> f32 {
        let sx = (ax + angle.cos() * SENSOR_DIST).floor() as i32;
        let sy = (ay + angle.sin() * SENSOR_DIST).floor() as i32;
        match Self::idx(sx, sy) {
            Some(i) => self.structure[i] * 0.5 + self.trail[i],
            None => 0.0,
        }
    }

    fn step_agents(&mut self, p: &Params) {
        for i in 0..self.agents.len() {
            let mut a = self.agents[i].clone();
            let s_l = self.sense(a.x, a.y, a.angle - p.sensor_angle);
            let s_c = self.sense(a.x, a.y, a.angle);
            let s_r = self.sense(a.x, a.y, a.angle + p.sensor_angle);
            let turn = 0.08 + self.rng.next() * 0.06;
            if s_c >= s_l && s_c >= s_r {
                a.angle += (self.rng.next() - 0.5) * 0.3;
            } else if s_l > s_r {
                a.angle -= turn;
            } else {
                a.angle += turn;
            }
            let speed = a.speed * p.flow;
            a.x = (a.x + a.angle.cos() * speed).rem_euclid(COLS as f32);
            a.y = (a.y + a.angle.sin() * speed).rem_euclid(ROWS as f32);
            let (mut gx, mut gy) = (a.x as i32, a.y as i32);
            // Off the network: re-seed onto it so density stays even.
            if let Some(idx) = Self::idx(gx, gy)
                && self.structure[idx] < 0.05
            {
                for _ in 0..30 {
                    let rx = (self.rng.next() * COLS as f32) as i32;
                    let ry = (self.rng.next() * ROWS as f32) as i32;
                    if let Some(ri) = Self::idx(rx, ry)
                        && self.structure[ri] > 0.1
                    {
                        a.x = rx as f32;
                        a.y = ry as f32;
                        a.angle = self.rng.next() * std::f32::consts::TAU;
                        a.ci = self.color_idx[ri];
                        gx = rx;
                        gy = ry;
                        break;
                    }
                }
            }
            if let Some(idx) = Self::idx(gx, gy) {
                self.trail[idx] = (self.trail[idx] + 0.04).min(1.0);
                self.color_idx[idx] = a.ci;
            }
            self.agents[i] = a;
        }
    }

    fn decay_trail(&mut self, retention: f32) {
        for t in self.trail.iter_mut() {
            *t *= retention;
            if *t < 0.005 {
                *t = 0.0;
            }
        }
    }

    fn reset(&mut self, p: &Params) {
        self.structure.fill(0.0);
        self.trail.fill(0.0);
        self.color_idx.fill(0);
        self.tips.clear();
        self.agents.clear();
        self.rng = Rng(0x9E37_79B9);
        for _ in 0..p.colonies.max(1) {
            let x = 15.0 + self.rng.next() * (COLS as f32 - 30.0);
            let y = 15.0 + self.rng.next() * (ROWS as f32 - 30.0);
            let ci = (self.rng.next() * 3.0) as u8;
            let n = 2 + (self.rng.next() * 3.0) as usize;
            self.seed_colony(x, y, ci, n);
        }
        // Pre-warm so nobody watches it sprout from nothing.
        let warm = Params {
            growth: 1.0,
            branching: 0.015,
            wander: 0.1,
            ..p.clone()
        };
        for _ in 0..4000 {
            self.grow_step(&warm);
        }
        for _ in 0..p.agents.max(0) {
            self.spawn_agent();
        }
        for _ in 0..200 {
            self.decay_trail(0.995);
            self.step_agents(&warm);
        }
    }

    fn step(&mut self, p: &Params) {
        self.grow_step(p);
        if self.tips.len() < 8 {
            let x = 10.0 + self.rng.next() * (COLS as f32 - 20.0);
            let y = 10.0 + self.rng.next() * (ROWS as f32 - 20.0);
            let ci = (self.rng.next() * 3.0) as u8;
            let n = 2 + (self.rng.next() * 2.0) as usize;
            self.seed_colony(x, y, ci, n);
        }
        self.decay_trail(p.trail_decay);
        // Track the requested agent count.
        let wanted = p.agents.max(0) as usize;
        while self.agents.len() < wanted {
            let before = self.agents.len();
            self.spawn_agent();
            if self.agents.len() == before {
                break;
            }
        }
        self.agents.truncate(wanted);
        self.step_agents(p);
    }

    fn paint(&self, p: &Params, pixels: &mut [u8]) {
        let bg = p.background;
        let ink = p.ink;
        // Three depths of one accent: enough variation to read as separate
        // colonies without spending a second hue.
        let mix = |t: f32| -> [f32; 3] {
            [
                ink[0] + (bg[0] - ink[0]) * t,
                ink[1] + (bg[1] - ink[1]) * t,
                ink[2] + (bg[2] - ink[2]) * t,
            ]
        };
        let palette = [mix(0.0), mix(0.28), mix(0.5)];
        for (i, px) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let s = self.structure[i];
            let t = self.trail[i];
            let alpha = (s * p.structure_gain + t * p.flow_gain).clamp(0.0, 1.0);
            let col = palette[(self.color_idx[i] as usize).min(2)];
            for c in 0..3 {
                let v = bg[c] + (col[c] - bg[c]) * alpha;
                px[c] = (v.clamp(0.0, 1.0) * 255.0) as u8;
            }
            px[3] = 255;
        }
    }
}

impl RustSource for Mycelium {
    const NAME: &'static str = "mycelium";
    const DOC: &'static str = "Hyphal network grown by wandering tips, traced by nutrient agents";
    const WIDTH: u32 = COLS as u32;
    const HEIGHT: u32 = ROWS as u32;
    const NEAREST: bool = true;
    type Params = Params;

    fn new() -> Self {
        let mut sim = Self {
            structure: vec![0.0; COLS * ROWS],
            trail: vec![0.0; COLS * ROWS],
            color_idx: vec![0; COLS * ROWS],
            tips: Vec::new(),
            agents: Vec::new(),
            rng: Rng(0x9E37_79B9),
            step_debt: 0.0,
            reset_latch: false,
        };
        sim.reset(&Params::default());
        sim
    }

    fn update(&mut self, p: &Params, frame: &FrameInfo, pixels: &mut [u8]) {
        if p.reset && !self.reset_latch {
            self.reset(p);
        }
        self.reset_latch = p.reset;
        // Fixed-rate stepping on the transport clock: paused → no steps.
        self.step_debt += frame.dt * p.sim_rate;
        let mut steps = 0;
        while self.step_debt >= 1.0 && steps < 8 {
            self.step(p);
            self.step_debt -= 1.0;
            steps += 1;
        }
        if steps == 8 {
            self.step_debt = 0.0;
        }
        self.paint(p, pixels);
    }
}
