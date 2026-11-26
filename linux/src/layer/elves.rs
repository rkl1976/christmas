//! Elves (Danish "nisser": red hat, grey clothes) walking on the snow pile. They stop, wave, carry gifts, meet to
//! hand over or swap a gift, and walk over to the neighbouring screen. Every 1½–3 minutes one arrives or leaves,
//! so their number swings a little around the chosen count. Ported from the "nisser" part of `src/Layer.cs`.

use cairo::LineCap;

use super::draw::*;
use super::{heap_at, Layer, SceneState};
use crate::folk::Folk;

const TUNICS: [Argb; 4] = [rgb(115, 120, 130), rgb(40, 105, 65), rgb(150, 35, 45), rgb(70, 85, 145)];
const GIFTS: [Argb; 4] = [rgb(40, 140, 70), rgb(60, 100, 200), rgb(200, 40, 60), rgb(150, 80, 180)];
const MEET_TIME: f32 = 2.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfState {
    Walk,
    Stand,
    Wave,
}

/// One elf. It moves between screens as a whole (same clothes, direction and gift).
pub struct Elf {
    id: u64,
    pub(super) x: f32,
    dir: f32,
    speed: f32,
    walk_phase: f32,
    t: f32,
    pub(super) state: ElfState,
    pub(super) state_timer: f32,
    size: f32,
    carries_gift: bool,
    /// Walking in from an edge, walking out for good, or walking over to the neighbouring screen.
    entering: bool,
    leaving: bool,
    crossing: bool,
    /// Meeting another elf to hand over or swap a gift; the one with `meet_with` makes the exchange.
    meet_timer: f32,
    meet_cooldown: f32,
    meet_with: Option<u64>,
    /// Magic past the children's snowman: gone for `magic` seconds, then back at `magic_to`. `zone_bumps` counts
    /// the turns at the play zone since the last time.
    magic: f32,
    magic_to: f32,
    zone_bumps: i32,
    tunic: Argb,
    mitten: Argb,
    gift: Argb,
}

impl Elf {
    fn free_to_meet(&self) -> bool {
        self.state == ElfState::Walk
            && self.magic <= 0.0
            && self.meet_timer <= 0.0
            && self.meet_cooldown <= 0.0
            && !self.entering
            && !self.leaving
            && !self.crossing
    }

    /// Which way it walks: -1 left, 1 right (used to hand it to the neighbouring screen).
    pub fn dir(&self) -> f32 {
        self.dir
    }

    /// Walking over to the neighbouring screen by its own choice (not sent away): its place goes with it.
    pub fn crossing(&self) -> bool {
        self.crossing
    }
}

impl Layer {
    /// All elves wave for a while (the elf party behind some calendar doors).
    pub fn elf_wave(&mut self, seconds: f32) {
        for e in &mut self.elves {
            e.state = ElfState::Wave;
            e.state_timer = seconds;
        }
    }

    /// Elves that walked off this screen towards a neighbour since the last call.
    pub fn take_outgoing_elves(&mut self) -> Vec<Elf> {
        std::mem::take(&mut self.outgoing)
    }

    /// Elves on their way over to the neighbour in that direction: decided, not handed over yet.
    pub fn elves_crossing(&self, dir: f32) -> i32 {
        self.elves.iter().filter(|e| e.crossing && e.dir * dir > 0.0).count() as i32
    }

    /// An elf arriving from the neighbouring screen walks in at the matching edge. One that was sent away keeps
    /// walking, across this screen and out (it is not counted here).
    pub fn receive_elf(&mut self, mut e: Elf, from_scale: f32) {
        let s = self.s;
        e.crossing = false;
        e.entering = true;
        e.state = ElfState::Walk;
        e.speed = e.speed / from_scale * s;
        e.x = if e.dir > 0.0 { -40.0 * s } else { self.w as f32 + 40.0 * s };
        e.state_timer = 4.0 + self.rng.f() * 6.0;
        e.meet_with = None;
        self.elves.push(e);
        crate::log::debug(&format!(
            "elf crossed to screen x={} dir={} ({} here now)",
            self.rect.x,
            self.elves.last().map_or(0.0, |e| e.dir),
            self.elves.len()
        ));
    }

    fn add_elf(&mut self, walking_in: bool) {
        let (s, w) = (self.s, self.w as f32);
        let r = &mut self.rng;
        let dir = if r.f() < 0.5 { -1.0 } else { 1.0 };
        let x = if walking_in {
            if dir > 0.0 {
                -40.0 * s
            } else {
                w + 40.0 * s
            }
        } else {
            60.0 * s + r.f() * (w - 120.0 * s)
        };
        let speed = (22.0 + r.f() * 18.0) * s;
        let size = 0.85 + r.f() * 0.3;
        let tunic = TUNICS[r.range(0, TUNICS.len() as i32) as usize];
        let mitten = if tunic.1 > 140 { rgb(40, 105, 65) } else { HAT_RED };
        let carries_gift = r.f() < 0.4;
        let gift = GIFTS[r.range(0, GIFTS.len() as i32) as usize];
        let state_timer = 4.0 + r.f() * 6.0;
        let t = r.f() * 10.0;
        self.next_elf_id += 1;
        self.elves.push(Elf {
            id: self.next_elf_id,
            x,
            dir,
            speed,
            walk_phase: 0.0,
            t,
            state: ElfState::Walk,
            state_timer,
            size,
            carries_gift,
            entering: walking_in,
            leaving: false,
            crossing: false,
            meet_timer: 0.0,
            meet_cooldown: 0.0,
            meet_with: None,
            magic: 0.0,
            magic_to: 0.0,
            zone_bumps: 0,
            tunic,
            mitten,
            gift,
        });
    }

    pub(super) fn update_elves(&mut self, dt: f32, st: &SceneState) {
        let (s, w) = (self.s, self.w as f32);
        // Every 1½–3 minutes an elf arrives or leaves. The offset is pulled back toward 0, so the number swings
        // a little up and down but stays around the chosen count over time.
        self.elf_change_timer -= dt;
        if self.elf_change_timer <= 0.0 {
            // JULE_ELF_SECONDS=N (or JULE_NISSE_SECONDS) changes the interval, for testing
            self.elf_change_timer = elf_seconds().unwrap_or_else(|| 90.0 + self.rng.f() * 90.0);
            let o = self.elf_offset;
            self.elf_offset = if o == 0 {
                if self.rng.f() < 0.5 {
                    -1
                } else {
                    1
                }
            } else if self.rng.f() < 0.7 {
                o - o.signum()
            } else {
                (o + o.signum()).clamp(-2, 2)
            };
        }
        // a lent or borrowed place is forgotten when there is no neighbour any more (or no elves)
        if !st.elves || (!self.neighbor_left && !self.neighbor_right) {
            self.elf_balance = 0;
        }
        let target = if st.elves { ((st.elf_count + self.elf_offset).max(1) + self.elf_balance).max(1) as usize } else { 0 };
        let staying = self.elves.iter().filter(|e| !e.leaving).count();
        if staying < target {
            // At start-up everyone is already there; later arrivals walk in from a screen edge.
            let arriving = !self.elves.is_empty() || st.time > 2.0;
            for _ in 0..if arriving { 1 } else { target - staying } {
                self.add_elf(arriving);
            }
        } else if staying > target {
            if !st.elves {
                self.elves.clear();
            } else if let Some(e) = self.elves.iter_mut().rev().find(|e| !e.leaving && !e.entering && !e.crossing) {
                // Walks off at an edge with no screen beyond it if there is one (so the count really goes down),
                // otherwise at the nearest edge, where it crosses to the neighbouring screen.
                e.leaving = true;
                e.state = ElfState::Walk;
                let (left_free, right_free) = (!self.neighbor_left, !self.neighbor_right);
                e.dir = if left_free != right_free {
                    if left_free {
                        -1.0
                    } else {
                        1.0
                    }
                } else if e.x < w / 2.0 {
                    -1.0
                } else {
                    1.0
                };
            }
        }

        // New Year: in the last minutes before midnight the guests on the clock's screen gather round it, each at
        // its own place to one side, and stay there (jumping when the controller says so) until it is over.
        let gather = if st.gather && self.primary { self.snowman_pos.map(|(x, _)| x - self.rect.x as f32) } else { None };
        if gather.is_none() {
            self.meet_elves();
        }
        let zone = self.play_zone(st);
        // where an elf vanished or reappeared by magic (x, size)
        let mut puffs: Vec<(f32, f32)> = Vec::new();
        let mut exchanges = Vec::new();
        // An elf walks over to the neighbour only while this screen has not already lent one, keeps at least one
        // itself, and the neighbour has no guest already.
        let mut going = self.elves.iter().filter(|e| e.crossing).count() as i32;
        let (balance, places) = (self.elf_balance, target as i32);
        let (room_left, room_right) = (self.guest_room_left, self.guest_room_right);
        for i in 0..self.elves.len() {
            let e = &mut self.elves[i];
            e.meet_cooldown = (e.meet_cooldown - dt).max(0.0);
            if let Some(cx) = gather.filter(|_| e.magic <= 0.0 && e.meet_timer <= 0.0 && !e.entering && !e.leaving && !e.crossing) {
                let side = if i % 2 == 0 { -1.0 } else { 1.0 };
                let slot = (cx + side * (62.0 + (i / 2) as f32 * 24.0) * s).max(30.0 * s).min(w - 30.0 * s);
                e.t += dt;
                if e.t > 200.0 * std::f32::consts::PI {
                    e.t -= 200.0 * std::f32::consts::PI;
                }
                if e.state == ElfState::Wave {
                    e.state_timer -= dt;
                    if e.state_timer <= 0.0 {
                        e.state = ElfState::Stand;
                    }
                } else if (e.x - slot).abs() > 4.0 * s {
                    e.state = ElfState::Walk;
                    e.dir = if slot > e.x { 1.0 } else { -1.0 };
                    e.x += e.dir * e.speed * 1.6 * dt;
                    e.walk_phase += dt * e.speed * 1.6 / (4.5 * s);
                } else {
                    e.state = ElfState::Stand;
                    e.dir = if cx > e.x { 1.0 } else { -1.0 };
                }
                continue;
            }
            if e.magic > 0.0 {
                // vanished by magic: it reappears on the other side of the children, walking on
                e.magic -= dt;
                if e.magic <= 0.0 {
                    e.x = e.magic_to;
                    puffs.push((e.x, e.size));
                }
                continue;
            }
            if e.meet_timer > 0.0 {
                let before = e.meet_timer;
                e.meet_timer -= dt;
                // halfway through the meeting the gift changes hands
                if before > MEET_TIME / 2.0 && e.meet_timer <= MEET_TIME / 2.0 {
                    if let Some(other) = e.meet_with {
                        exchanges.push((e.id, other));
                    }
                }
                if e.meet_timer <= 0.0 {
                    // say goodbye and walk on, each in its own direction (past each other)
                    e.meet_with = None;
                    e.state = ElfState::Walk;
                    e.state_timer = 4.0 + self.rng.f() * 6.0;
                    e.meet_cooldown = 20.0;
                }
                e.t += dt;
                continue;
            }
            // Wrap at 200π (a whole number of periods for both the wave and the hat sway) so the float never grows
            // large enough to lose precision during long uptimes.
            e.t += dt;
            if e.t > 200.0 * std::f32::consts::PI {
                e.t -= 200.0 * std::f32::consts::PI;
            }
            e.state_timer -= dt;
            if e.state == ElfState::Walk {
                // The elves keep out of the way where the children build their snowman: they turn before it (one
                // that is already inside when they begin just walks on out). One that is trapped behind the
                // children (a narrow strip to the screen edge, or back at the children a second time) vanishes
                // by magic and reappears on the other side, if there is room there.
                if let Some((zl, zr)) = zone {
                    let ahead = e.x + e.dir * 14.0 * s;
                    if !e.entering && (e.x < zl || e.x > zr) && ahead >= zl && ahead <= zr {
                        let to = if e.dir > 0.0 { zr + 20.0 * s } else { zl - 20.0 * s };
                        let behind = if e.dir > 0.0 { zl } else { w - zr };
                        let room = to > 45.0 * s && to < w - 45.0 * s;
                        if room && (e.zone_bumps >= 1 || behind < 250.0 * s) {
                            e.zone_bumps = 0;
                            puffs.push((e.x, e.size));
                            e.magic = 0.8;
                            e.magic_to = to;
                            crate::log::debug(&format!("elf magic past the children: x={} to {}", e.x as i32, to as i32));
                            continue;
                        }
                        e.zone_bumps += 1;
                        e.dir = -e.dir;
                    }
                }
                e.x += e.dir * e.speed * dt;
                e.walk_phase += dt * e.speed / (4.5 * s);
                if e.entering && e.x > 30.0 * s && e.x < w - 30.0 * s {
                    e.entering = false;
                }
                if !e.entering && !e.leaving && !e.crossing {
                    // At an edge with a neighbouring screen, about half the elves walk on over to it.
                    let free = balance - going > -1 && places - going > 1;
                    if e.x < 30.0 * s {
                        if self.neighbor_left && self.rng.f() < 0.5 && free && room_left {
                            e.crossing = true;
                            going += 1;
                        } else {
                            e.x = 30.0 * s;
                            e.dir = 1.0;
                        }
                    }
                    if e.x > w - 30.0 * s {
                        if self.neighbor_right && self.rng.f() < 0.5 && free && room_right {
                            e.crossing = true;
                            going += 1;
                        } else {
                            e.x = w - 30.0 * s;
                            e.dir = -1.0;
                        }
                    }
                }
                // no stopping or turning while arriving, leaving or crossing to another screen
                if e.entering || e.leaving || e.crossing {
                    continue;
                }
                if e.state_timer <= 0.0 {
                    let r = self.rng.f();
                    if r < 0.3 {
                        e.state = ElfState::Stand;
                        e.state_timer = 2.0 + self.rng.f() * 3.0;
                    } else if r < 0.5 {
                        e.state = ElfState::Wave;
                        e.state_timer = 2.5;
                    } else {
                        if r < 0.7 {
                            e.dir = -e.dir;
                        }
                        e.state_timer = 4.0 + self.rng.f() * 8.0;
                    }
                }
            } else if e.leaving {
                e.state = ElfState::Walk; // stop waving and keep walking out
            } else if e.state_timer <= 0.0 {
                e.state = ElfState::Walk;
                e.state_timer = 4.0 + self.rng.f() * 8.0;
                if self.rng.f() < 0.4 {
                    e.dir = -e.dir;
                }
            }
        }
        for (a, b) in exchanges {
            self.exchange_gifts(a, b);
        }
        for (x, size) in puffs {
            let gy = self.ground_y - heap_at(&self.heap, x.clamp(0.0, w - 1.0)) + 2.0 * s;
            self.magic_puffs.push((self.rect.x as f32 + x, self.rect.y as f32 + gy, s * size));
        }

        // Elves that walked off the screen continue on the neighbouring screen in that direction, if there is one
        // (the controller hands them over); otherwise they are gone.
        let mut i = self.elves.len();
        while i > 0 {
            i -= 1;
            let e = &self.elves[i];
            if !(e.leaving || e.crossing) || (e.x >= -45.0 * s && e.x <= w + 45.0 * s) {
                continue;
            }
            let e = self.elves.remove(i);
            let has_next = if e.dir > 0.0 { self.neighbor_right } else { self.neighbor_left };
            if has_next && st.elves {
                self.outgoing.push(e);
            }
        }
    }

    /// Two elves walking toward each other stop when they meet, if at least one of them carries a gift.
    fn meet_elves(&mut self) {
        let s = self.s;
        for a in 0..self.elves.len() {
            if !self.elves[a].free_to_meet() {
                continue;
            }
            for b in 0..self.elves.len() {
                let (ea, eb) = (&self.elves[a], &self.elves[b]);
                if a == b || !eb.free_to_meet() || ea.dir == eb.dir || !(ea.carries_gift || eb.carries_gift) {
                    continue;
                }
                let d = eb.x - ea.x;
                // b is in front of a, close enough to reach out
                if d * ea.dir <= 0.0 || d.abs() >= 24.0 * s * (ea.size + eb.size) / 2.0 {
                    continue;
                }
                let id_b = eb.id;
                for (i, with) in [(a, Some(id_b)), (b, None)] {
                    let e = &mut self.elves[i];
                    e.meet_timer = MEET_TIME;
                    e.state = ElfState::Stand;
                    e.meet_with = with;
                }
                return; // one meeting at a time is plenty
            }
        }
    }

    /// One gift: it goes to the other elf. Two gifts: they swap.
    fn exchange_gifts(&mut self, a: u64, b: u64) {
        let (Some(ia), Some(ib)) =
            (self.elves.iter().position(|e| e.id == a), self.elves.iter().position(|e| e.id == b))
        else {
            return;
        };
        let (ga, gb) =
            ((self.elves[ia].carries_gift, self.elves[ia].gift), (self.elves[ib].carries_gift, self.elves[ib].gift));
        let swapped = ga.0 && gb.0;
        if swapped {
            self.elves[ia].gift = gb.1;
            self.elves[ib].gift = ga.1;
        } else {
            let (giver, taker) = if ga.0 { (ia, ib) } else { (ib, ia) };
            let gift = self.elves[giver].gift;
            self.elves[taker].carries_gift = true;
            self.elves[taker].gift = gift;
            self.elves[giver].carries_gift = false;
        }
        crate::log::debug(&format!("elves met: gift {}", if swapped { "swapped" } else { "handed over" }));
    }

    /// `folk`: who the walkers are in the Christmas theme (setting `Folk`).
    pub(super) fn draw_elves(&mut self, st: &SceneState, folk: Folk) {
        for i in 0..self.elves.len() {
            // (one that is gone by magic for a moment is not drawn)
            if self.elves[i].magic <= 0.0 {
                self.draw_elf(i, st, folk);
            }
        }
    }

    fn draw_elf(&mut self, i: usize, st: &SceneState, folk: Folk) {
        let (autumn, muertos, halloween) = (st.autumn, st.muertos, st.halloween);
        let e = &self.elves[i];
        let sc = self.s * e.size;
        let gy = self.ground_y - heap_at(&self.heap, e.x) + 2.0 * self.s;
        let (ex, dir) = (e.x, e.dir);
        if st.karneval() {
            // (carnival's flag bearer holds her flag higher and farther out than a nisse reaches)
            self.mark(ex - 38.0 * sc, gy - 84.0 * sc, 76.0 * sc, 92.0 * sc);
        } else {
            self.mark(ex - 32.0 * sc, gy - 72.0 * sc, 64.0 * sc, 80.0 * sc);
        }
        let e = &self.elves[i];

        let cr = &self.canvas.cr;
        let saved = cr.matrix();
        cr.translate(ex as f64, gy as f64);
        cr.scale((dir * sc) as f64, sc as f64);
        if autumn {
            animal(cr, e);
            cr.set_matrix(saved);
            return;
        }
        if muertos {
            // De dødes dag: a calaca (`muertos.rs`), in the same pose as the elf it stands in for
            let walking = e.state == ElfState::Walk;
            let waving = e.state == ElfState::Wave;
            super::muertos::calaca(
                cr,
                super::muertos::CalacaPose {
                    tunic: tunic_index(e),
                    gift: GIFTS.iter().position(|g| *g == e.gift).unwrap_or(0),
                    walking,
                    waving,
                    carries_gift: e.carries_gift,
                    walk: if walking { e.walk_phase as f64 } else { 0.0 },
                    stand: if e.state == ElfState::Stand { e.t as f64 * 1.5 } else { 0.0 },
                    wave: if waving { e.t as f64 * 11.0 } else { 0.0 },
                },
            );
            cr.set_matrix(saved);
            return;
        }
        if st.nytaar {
            // New Year: a party guest (`nytaar.rs`), in the same pose as the elf it stands in for. Who it is follows
            // the elf itself, not what it carries: a guest keeps its look when a cracker changes hands.
            let walking = e.state == ElfState::Walk;
            let waving = e.state == ElfState::Wave;
            super::nytaar::guest(
                cr,
                super::nytaar::GuestPose {
                    look: (e.id % 8) as usize,
                    gift: GIFTS.iter().position(|g| *g == e.gift).unwrap_or(0),
                    walking,
                    waving,
                    carries_gift: e.carries_gift,
                    walk: if walking { e.walk_phase as f64 } else { 0.0 },
                    stand: if e.state == ElfState::Stand { e.t as f64 * 1.5 } else { 0.0 },
                    wave: if waving { e.t as f64 * 11.0 } else { 0.0 },
                },
            );
            cr.set_matrix(saved);
            return;
        }
        if st.karneval() {
            // Carnival: one of a samba parade, or of the Rhineland's (`karneval.rs`), in the same state as the elf
            // it stands in for. Who it is follows the elf itself, and its step the controller's beat, not the
            // elf's own phase: everyone on every screen moves together.
            super::karneval::reveller(
                cr,
                &super::karneval::RevellerPose {
                    look: (e.id % 9973) as usize,
                    tunic: tunic_index(e),
                    gift: e.gift,
                    walking: e.state == ElfState::Walk,
                    standing: e.state == ElfState::Stand,
                    waving: e.state == ElfState::Wave,
                    carries_gift: e.carries_gift,
                    beat_count: st.beat_count,
                },
                st.carnival == crate::folk::CARNIVAL_RHEIN,
            );
            cr.set_matrix(saved);
            return;
        }
        if halloween {
            // Halloween: a ghost or a child dressed up (`halloween.rs`), in the same pose as the elf it stands in for
            let walking = e.state == ElfState::Walk;
            let waving = e.state == ElfState::Wave;
            super::halloween::spook(
                cr,
                super::halloween::SpookPose {
                    tunic: tunic_index(e),
                    walking,
                    waving,
                    carries_gift: e.carries_gift,
                    walk: if walking { e.walk_phase as f64 } else { 0.0 },
                    stand: if e.state == ElfState::Stand { e.t as f64 * 1.5 } else { 0.0 },
                    wave: if waving { e.t as f64 * 11.0 } else { 0.0 },
                },
            );
            cr.set_matrix(saved);
            return;
        }
        // every folk but the nisse (which follows below) has its own function
        match folk {
            Folk::Nisser => {}
            Folk::Alfer => alf(cr, e),
            Folk::Wichtel => wichtel(cr, e),
            Folk::Kabouter => kabouter(cr, e),
            Folk::Paekapikk => paekapikk(cr, e),
        }
        if folk != Folk::Nisser {
            cr.set_matrix(saved);
            return;
        }

        let walking = e.state == ElfState::Walk;
        let p = e.walk_phase as f64;
        let leg_a = if walking { p.sin() * 0.55 } else { 0.0 };
        let bob = if walking { -p.sin().abs() * 1.5 } else { 0.0 };

        fill_ellipse(cr, SHADOW, -11.0, -2.0, 22.0, 4.0);

        let pants = rgb(80, 80, 92);
        let boot = rgb(62, 40, 28);
        let leg = |hx: f64, hy: f64, a: f64| {
            let (fx, fy) = (hx + a.sin() * 12.0, hy + a.cos() * 12.0);
            line(cr, pants, 5.0, LineCap::Round, hx, hy, fx, fy);
            fill_ellipse(cr, boot, fx - 3.0, fy - 2.0, 9.0, 5.0);
        };
        leg(-2.0, -15.0 + bob, leg_a);
        leg(2.0, -15.0 + bob, -leg_a);

        cr.translate(0.0, bob);
        let (back_arm, front_arm) = if e.state == ElfState::Wave {
            (-leg_a * 0.8, 2.5 + (e.t as f64 * 11.0).sin() * 0.45)
        } else if e.carries_gift {
            (1.3, 1.3)
        } else {
            (-leg_a * 0.9, leg_a * 0.9)
        };
        let arm = |sx: f64, sy: f64, a: f64| {
            let (hx, hy) = (sx + a.sin() * 10.0, sy + a.cos() * 10.0);
            line(cr, e.tunic, 4.2, LineCap::Round, sx, sy, hx, hy);
            fill_ellipse(cr, e.mitten, hx - 2.5, hy - 2.5, 5.0, 5.0);
        };

        arm(-1.0, -29.0, back_arm);
        fill_ellipse(cr, e.tunic, -10.0, -34.0, 20.0, 22.0);
        fill_rect(cr, rgb(35, 30, 30), -9.5, -20.0, 19.0, 3.0);
        fill_rect(cr, GOLD, 2.0, -20.5, 3.5, 4.0);

        if e.carries_gift && e.state != ElfState::Wave {
            fill_rect(cr, e.gift, 6.0, -31.0, 11.0, 10.0);
            line(cr, GOLD, 1.6, LineCap::Butt, 11.5, -31.0, 11.5, -21.0);
            line(cr, GOLD, 1.6, LineCap::Butt, 6.0, -26.0, 17.0, -26.0);
            draw_ellipse(cr, GOLD, 1.6, 9.0, -34.0, 3.0, 3.0);
            draw_ellipse(cr, GOLD, 1.6, 12.0, -34.0, 3.0, 3.0);
        }

        fill_ellipse(cr, SKIN, -6.0, -44.0, 13.0, 12.0);
        cr.new_path();
        cr.move_to(-6.0, -39.0);
        cr.curve_to(-7.0, -30.0, -2.0, -22.0, 3.0, -20.0);
        cr.curve_to(7.0, -24.0, 9.0, -32.0, 7.5, -39.0);
        cr.close_path();
        set(cr, rgb(250, 250, 250));
        let _ = cr.fill();
        arm(1.0, -29.0, front_arm);
        fill_ellipse(cr, rgb(240, 160, 150), 4.0, -40.0, 5.0, 5.0);

        let sway = (if walking { p * 0.5 } else { e.t as f64 * 1.5 }).sin() * 2.5;
        let (tip_x, tip_y) = (-14.0 + sway, -57.0);
        cr.new_path();
        cr.move_to(-7.5, -39.0);
        cr.line_to(8.0, -39.0);
        cr.curve_to(9.0, -54.0, 0.0, -62.0, tip_x, tip_y);
        cr.curve_to(-6.0, -57.0, -7.0, -49.0, -7.5, -39.0);
        cr.close_path();
        set(cr, HAT_RED);
        let _ = cr.fill();
        line(cr, rgb(150, 15, 25), 2.0, LineCap::Butt, -7.5, -39.5, 8.0, -39.5);
        fill_ellipse(cr, WHITE, tip_x - 3.0, tip_y - 3.0, 6.0, 6.0);
        cr.set_matrix(saved);
    }
}

/// The Christmas elf, for the parts of the world where a nisse means nothing (see `folk.rs`; `DrawElf` on Windows,
/// `drawElf` on macOS). It has the nisse's skeleton and poses, so walking, waving, meeting and gifts are the same;
/// only the looks differ: a green or red tunic with a jagged collar and hem, striped stockings, shoes with a curled
/// toe, a pointed ear, and a pointed cap with a bell where the nisse has its pompom. No beard. An original figure
/// like the nisse: no known character. Which clothes follows the nisse's own, so a walker keeps its colours.
/// Drawn in the elf's frame: feet at (0, 0), y up is negative, facing right.
const ALF_TUNICS: [Argb; 4] = [rgb(46, 139, 74), rgb(196, 42, 52), rgb(30, 110, 92), rgb(112, 158, 58)];
/// The colour of collar, cap band and mittens that goes with each tunic.
const ALF_TRIMS: [Argb; 4] = [rgb(205, 40, 50), rgb(46, 139, 74), rgb(235, 195, 70), rgb(205, 40, 50)];

/// A filled shape of curves: the start point, then two control points and the end per curve.
fn fill_curves(cr: &cairo::Context, col: Argb, start: (f64, f64), xy: &[f64]) {
    cr.new_path();
    cr.move_to(start.0, start.1);
    for c in xy.chunks_exact(6) {
        cr.curve_to(c[0], c[1], c[2], c[3], c[4], c[5]);
    }
    cr.close_path();
    set(cr, col);
    let _ = cr.fill();
}

fn alf(cr: &cairo::Context, e: &Elf) {
    let walking = e.state == ElfState::Walk;
    let p = e.walk_phase as f64;
    let leg_a = if walking { p.sin() * 0.55 } else { 0.0 };
    let bob = if walking { -p.sin().abs() * 1.5 } else { 0.0 };
    let ti = TUNICS.iter().position(|t| *t == e.tunic).unwrap_or(0) % ALF_TUNICS.len();
    let (tunic, trim) = (ALF_TUNICS[ti], ALF_TRIMS[ti]);
    let stripe = if ti == 1 { ALF_TUNICS[0] } else { rgb(205, 40, 50) }; // the red tunic gets green stripes

    fill_ellipse(cr, SHADOW, -11.0, -2.0, 22.0, 4.0);

    // a striped stocking and a shoe with a curled toe and a bell on its tip
    let leg = |hx: f64, hy: f64, a: f64| {
        let (dx, dy) = (a.sin(), a.cos());
        let (fx, fy) = (hx + dx * 12.0, hy + dy * 12.0);
        line(cr, rgb(248, 246, 240), 4.6, LineCap::Round, hx, hy, fx, fy);
        for k in [0.34_f64, 0.62, 0.9] {
            let (from, to) = (12.0 * (k - 0.09), 12.0 * (k + 0.09));
            line(cr, stripe, 4.6, LineCap::Butt, hx + dx * from, hy + dy * from, hx + dx * to, hy + dy * to);
        }
        fill_curves(cr, rgb(34, 92, 58), (fx - 3.5, fy + 3.0), &[
            fx - 4.5, fy - 1.5, fx + 0.5, fy - 3.0, fx + 4.5, fy + 0.3,
            fx + 6.5, fy + 1.5, fx + 8.5, fy - 0.5, fx + 8.8, fy - 3.8,
            fx + 11.5, fy + 0.5, fx + 9.0, fy + 3.0, fx + 6.0, fy + 3.0,
        ]);
        fill_ellipse(cr, GOLD, fx + 7.6, fy - 5.6, 2.4, 2.4);
    };
    leg(-2.0, -15.0 + bob, leg_a);
    leg(2.0, -15.0 + bob, -leg_a);

    cr.translate(0.0, bob);
    let (back_arm, front_arm) = if e.state == ElfState::Wave {
        (-leg_a * 0.8, 2.5 + (e.t as f64 * 11.0).sin() * 0.45)
    } else if e.carries_gift {
        (1.3, 1.3)
    } else {
        (-leg_a * 0.9, leg_a * 0.9)
    };
    let arm = |sx: f64, sy: f64, a: f64| {
        let (hx, hy) = (sx + a.sin() * 10.0, sy + a.cos() * 10.0);
        line(cr, tunic, 4.2, LineCap::Round, sx, sy, hx, hy);
        fill_ellipse(cr, trim, hx - 2.5, hy - 2.5, 5.0, 5.0);
    };

    arm(-1.0, -29.0, back_arm);
    fill_ellipse(cr, tunic, -9.0, -34.0, 18.0, 20.0);
    fill_polygon(cr, tunic, &[
        (-8.7, -19.0), (-9.5, -12.5), (-6.0, -15.5), (-3.0, -12.0), (0.0, -15.5), (3.0, -12.0), (6.0, -15.5),
        (9.5, -12.5), (8.7, -19.0),
    ]);
    fill_rect(cr, rgb(35, 30, 30), -8.8, -21.0, 17.6, 3.0);
    fill_rect(cr, GOLD, 1.5, -21.5, 3.5, 4.0);
    fill_polygon(cr, trim, &[
        (-7.0, -34.0), (-5.6, -28.5), (-3.0, -31.3), (-0.5, -27.8), (2.0, -31.3), (4.6, -28.5), (7.0, -34.0),
    ]);

    if e.carries_gift && e.state != ElfState::Wave {
        fill_rect(cr, e.gift, 6.0, -31.0, 11.0, 10.0);
        line(cr, GOLD, 1.6, LineCap::Butt, 11.5, -31.0, 11.5, -21.0);
        line(cr, GOLD, 1.6, LineCap::Butt, 6.0, -26.0, 17.0, -26.0);
        draw_ellipse(cr, GOLD, 1.6, 9.0, -34.0, 3.0, 3.0);
        draw_ellipse(cr, GOLD, 1.6, 12.0, -34.0, 3.0, 3.0);
    }

    fill_polygon(cr, SKIN, &[(-4.5, -36.0), (-11.5, -42.0), (-3.5, -41.0)]);
    line(cr, rgb(225, 170, 140), 0.8, LineCap::Butt, -5.2, -38.0, -9.0, -41.0);
    fill_ellipse(cr, SKIN, -6.0, -44.0, 13.0, 12.0);
    fill_ellipse(cr, (150, 245, 150, 140), 0.8, -36.4, 3.2, 2.4);
    fill_ellipse(cr, rgb(45, 32, 30), 2.7, -38.4, 1.7, 2.0);
    cr.new_path();
    cr.move_to(2.6, -34.4);
    cr.curve_to(3.6, -33.4, 5.0, -33.5, 5.9, -34.6);
    stroke(cr, rgb(150, 70, 60), 0.8, LineCap::Round);
    arm(1.0, -29.0, front_arm);
    fill_ellipse(cr, rgb(240, 160, 150), 5.2, -38.0, 3.2, 3.2);

    let sway = (if walking { p * 0.5 } else { e.t as f64 * 1.5 }).sin() * 2.5;
    let (tip_x, tip_y) = (-13.0 + sway, -60.0);
    cr.new_path();
    cr.move_to(-6.5, -41.0);
    cr.line_to(7.5, -41.0);
    cr.curve_to(7.5, -52.0, 1.0, -60.0, tip_x, tip_y);
    cr.curve_to(-6.0, -56.0, -6.5, -49.0, -6.5, -41.0);
    cr.close_path();
    set(cr, tunic);
    let _ = cr.fill();
    fill_polygon(cr, trim, &[
        (-7.5, -43.0), (8.5, -43.0), (8.5, -40.8), (6.5, -39.2), (4.5, -40.8), (2.5, -39.2), (0.5, -40.8),
        (-1.5, -39.2), (-3.5, -40.8), (-5.5, -39.2), (-7.5, -40.8),
    ]);
    fill_ellipse(cr, GOLD, tip_x - 2.6, tip_y - 2.6, 5.2, 5.2);
    line(cr, rgb(150, 110, 20), 0.9, LineCap::Butt, tip_x - 1.4, tip_y + 0.7, tip_x + 1.4, tip_y + 0.7);
}

// ---------------------------------------------------------------------- Wichtel, kabouter and päkapikk
// Three more looks for the same walker (see `folk.rs`), for the countries whose own figure is close to the nisse
// but not the same. Each has the nisse's skeleton and poses. Original figures: no known characters. Kept line for
// line the same as the macOS and Windows versions.
const WICHTEL_COATS: [Argb; 4] = [rgb(120, 122, 128), rgb(138, 44, 50), rgb(84, 108, 64), rgb(72, 86, 112)];
const WICHTEL_CAPS: [Argb; 4] = [rgb(150, 38, 46), rgb(96, 100, 108), rgb(150, 38, 46), rgb(176, 128, 52)];
const KABOUTER_JACKETS: [Argb; 4] = [rgb(62, 122, 82), rgb(190, 140, 52), rgb(152, 88, 52), rgb(70, 112, 140)];
const PAEKAPIKK_REDS: [Argb; 4] = [rgb(202, 36, 46), rgb(178, 32, 48), rgb(214, 62, 50), rgb(164, 36, 62)];

/// The walker's pose, the same for every folk.
struct Pose {
    leg_a: f64,
    bob: f64,
    back_arm: f64,
    front_arm: f64,
    /// -1 to 1: how far the cap's tip swings.
    sway: f64,
}

fn pose(e: &Elf) -> Pose {
    let walking = e.state == ElfState::Walk;
    let p = e.walk_phase as f64;
    let leg_a = if walking { p.sin() * 0.55 } else { 0.0 };
    let (back_arm, front_arm) = if e.state == ElfState::Wave {
        (-leg_a * 0.8, 2.5 + (e.t as f64 * 11.0).sin() * 0.45)
    } else if e.carries_gift {
        (1.3, 1.3)
    } else {
        (-leg_a * 0.9, leg_a * 0.9)
    };
    Pose {
        leg_a,
        bob: if walking { -p.sin().abs() * 1.5 } else { 0.0 },
        back_arm,
        front_arm,
        sway: (if walking { p * 0.5 } else { e.t as f64 * 1.5 }).sin(),
    }
}

fn tunic_index(e: &Elf) -> usize {
    TUNICS.iter().position(|t| *t == e.tunic).unwrap_or(0)
}

fn gift(cr: &cairo::Context, e: &Elf) {
    if !e.carries_gift || e.state == ElfState::Wave {
        return;
    }
    fill_rect(cr, e.gift, 6.0, -31.0, 11.0, 10.0);
    line(cr, GOLD, 1.6, LineCap::Butt, 11.5, -31.0, 11.5, -21.0);
    line(cr, GOLD, 1.6, LineCap::Butt, 6.0, -26.0, 17.0, -26.0);
    draw_ellipse(cr, GOLD, 1.6, 9.0, -34.0, 3.0, 3.0);
    draw_ellipse(cr, GOLD, 1.6, 12.0, -34.0, 3.0, 3.0);
}

/// The German Wichtel: a tall felt cap pulled down over the eyes, so only a round nose and a long beard show, and
/// a bell-shaped coat that nearly reaches the ground, with two boots under it.
fn wichtel(cr: &cairo::Context, e: &Elf) {
    let q = pose(e);
    let ti = tunic_index(e) % WICHTEL_COATS.len();
    let (coat, cap) = (WICHTEL_COATS[ti], WICHTEL_CAPS[ti]);

    fill_ellipse(cr, SHADOW, -12.0, -2.0, 24.0, 4.0);
    let leg = |hx: f64, hy: f64, a: f64| {
        let (fx, fy) = (hx + a.sin() * 6.0, hy + a.cos() * 6.0);
        line(cr, rgb(60, 50, 45), 4.5, LineCap::Round, hx, hy, fx, fy);
        fill_ellipse(cr, rgb(62, 40, 28), fx - 3.0, fy - 2.0, 9.0, 5.0);
    };
    leg(-2.5, -9.0 + q.bob, q.leg_a);
    leg(2.5, -9.0 + q.bob, -q.leg_a);

    cr.translate(0.0, q.bob);
    let arm = |sx: f64, sy: f64, a: f64| {
        let (hx, hy) = (sx + a.sin() * 9.0, sy + a.cos() * 9.0);
        line(cr, coat, 4.4, LineCap::Round, sx, sy, hx, hy);
        fill_ellipse(cr, rgb(70, 60, 55), hx - 2.5, hy - 2.5, 5.0, 5.0);
    };
    arm(-1.0, -27.0, q.back_arm);
    fill_curves(cr, coat, (-10.0, -7.0), &[-11.0, -20.0, -8.0, -31.0, 0.0, -32.0, 8.0, -31.0, 11.0, -20.0, 10.0, -7.0]);
    gift(cr, e);

    fill_ellipse(cr, SKIN, -6.0, -43.0, 13.0, 12.0);
    let beard = [-8.0, -24.0, -3.0, -13.0, 1.0, -9.0, 5.0, -13.0, 9.5, -24.0, 8.0, -37.0];
    fill_curves(cr, rgb(240, 240, 236), (-6.5, -37.0), &beard);
    arm(1.0, -27.0, q.front_arm);
    fill_ellipse(cr, rgb(235, 165, 150), 1.5, -39.0, 7.0, 7.0);

    let (tip_x, tip_y) = (-9.0 + q.sway * 2.5 * 1.6, -68.0);
    let hat = [9.0, -52.0, 2.0, -64.0, tip_x, tip_y, -7.0, -60.0, -9.0, -50.0, -9.5, -37.0];
    fill_curves(cr, cap, (10.0, -37.0), &hat);
    line(cr, (60, 0, 0, 0), 2.4, LineCap::Butt, -9.5, -37.8, 10.0, -37.8);
}

/// The Dutch kabouter: a stiff, straight cone hat, an open face with a short round beard, a jacket with a belt,
/// brown trousers and yellow clogs.
fn kabouter(cr: &cairo::Context, e: &Elf) {
    let q = pose(e);
    let jacket = KABOUTER_JACKETS[tunic_index(e) % KABOUTER_JACKETS.len()];
    let clog = rgb(232, 190, 80);

    fill_ellipse(cr, SHADOW, -11.0, -2.0, 22.0, 4.0);
    let leg = |hx: f64, hy: f64, a: f64| {
        let (fx, fy) = (hx + a.sin() * 12.0, hy + a.cos() * 12.0);
        line(cr, rgb(105, 75, 50), 5.0, LineCap::Round, hx, hy, fx, fy);
        fill_ellipse(cr, clog, fx - 3.5, fy - 2.0, 10.0, 5.0);
        fill_polygon(cr, clog, &[(fx + 3.5, fy - 1.6), (fx + 9.5, fy - 3.6), (fx + 6.5, fy + 2.4)]);
    };
    leg(-2.0, -15.0 + q.bob, q.leg_a);
    leg(2.0, -15.0 + q.bob, -q.leg_a);

    cr.translate(0.0, q.bob);
    let arm = |sx: f64, sy: f64, a: f64| {
        let (hx, hy) = (sx + a.sin() * 10.0, sy + a.cos() * 10.0);
        line(cr, jacket, 4.2, LineCap::Round, sx, sy, hx, hy);
        fill_ellipse(cr, SKIN, hx - 2.3, hy - 2.3, 4.6, 4.6);
    };
    arm(-1.0, -29.0, q.back_arm);
    fill_ellipse(cr, jacket, -10.0, -34.0, 20.0, 22.0);
    fill_rect(cr, rgb(60, 40, 25), -9.5, -20.0, 19.0, 3.0);
    fill_rect(cr, GOLD, 2.0, -20.5, 3.5, 4.0);
    gift(cr, e);

    fill_ellipse(cr, SKIN, -6.0, -44.0, 13.0, 12.0);
    let beard = [-7.5, -31.0, -3.0, -27.0, 1.5, -27.0, 6.0, -27.0, 9.0, -31.0, 7.5, -38.0];
    fill_curves(cr, rgb(250, 250, 250), (-6.0, -38.0), &beard);
    fill_ellipse(cr, (150, 245, 150, 140), 0.6, -39.6, 3.2, 2.4);
    fill_ellipse(cr, rgb(45, 32, 30), 2.8, -41.3, 1.6, 1.9);
    arm(1.0, -29.0, q.front_arm);
    fill_ellipse(cr, rgb(240, 160, 150), 4.2, -40.2, 4.0, 4.0);

    fill_polygon(cr, HAT_RED, &[(-7.5, -42.0), (8.0, -42.0), (0.5 + q.sway * 1.2, -67.0)]);
    line(cr, rgb(150, 15, 25), 1.8, LineCap::Butt, -7.5, -42.4, 8.0, -42.4);
}

/// The Estonian päkapikk: all in red with white trim, a short floppy cap with a fur band and a pompom, an open,
/// beardless face with fair hair showing under the cap.
fn paekapikk(cr: &cairo::Context, e: &Elf) {
    let q = pose(e);
    let red = PAEKAPIKK_REDS[tunic_index(e) % PAEKAPIKK_REDS.len()];
    let white = rgb(248, 248, 248);

    fill_ellipse(cr, SHADOW, -11.0, -2.0, 22.0, 4.0);
    let leg = |hx: f64, hy: f64, a: f64| {
        let (dx, dy) = (a.sin(), a.cos());
        let (fx, fy) = (hx + dx * 12.0, hy + dy * 12.0);
        line(cr, red, 4.8, LineCap::Round, hx, hy, fx, fy);
        line(cr, white, 4.8, LineCap::Butt, hx + dx * 8.2, hy + dy * 8.2, hx + dx * 10.2, hy + dy * 10.2);
        fill_ellipse(cr, rgb(40, 36, 40), fx - 3.0, fy - 2.0, 9.0, 5.0);
    };
    leg(-2.0, -15.0 + q.bob, q.leg_a);
    leg(2.0, -15.0 + q.bob, -q.leg_a);

    cr.translate(0.0, q.bob);
    let arm = |sx: f64, sy: f64, a: f64| {
        let (hx, hy) = (sx + a.sin() * 10.0, sy + a.cos() * 10.0);
        line(cr, red, 4.2, LineCap::Round, sx, sy, hx, hy);
        fill_ellipse(cr, white, hx - 2.5, hy - 2.5, 5.0, 5.0);
    };
    arm(-1.0, -29.0, q.back_arm);
    fill_ellipse(cr, red, -9.5, -34.0, 19.0, 21.0);
    line(cr, white, 3.0, LineCap::Round, -7.6, -15.2, 7.6, -15.2);
    fill_rect(cr, rgb(35, 30, 30), -9.2, -22.0, 18.4, 2.6);
    fill_rect(cr, GOLD, 1.8, -22.5, 3.3, 3.6);
    fill_ellipse(cr, white, -5.5, -34.6, 12.0, 4.2);
    gift(cr, e);

    fill_ellipse(cr, rgb(232, 192, 96), -7.2, -42.5, 6.0, 6.0);
    fill_ellipse(cr, SKIN, -6.0, -44.0, 13.0, 12.0);
    fill_ellipse(cr, rgb(240, 196, 168), -5.2, -39.6, 2.6, 3.4);
    fill_ellipse(cr, (150, 245, 150, 140), 0.8, -36.6, 3.2, 2.4);
    fill_ellipse(cr, rgb(45, 32, 30), 2.8, -38.8, 1.7, 2.0);
    cr.new_path();
    cr.move_to(2.6, -34.6);
    cr.curve_to(3.6, -33.6, 5.0, -33.7, 5.9, -34.8);
    stroke(cr, rgb(150, 70, 60), 0.8, LineCap::Round);
    arm(1.0, -29.0, q.front_arm);
    fill_ellipse(cr, rgb(240, 160, 150), 5.0, -38.3, 3.4, 3.4);

    let (tip_x, tip_y) = (-12.0 + q.sway * 2.5, -55.0);
    let hat = [8.5, -52.0, 0.0, -58.0, tip_x, tip_y, -6.0, -54.0, -7.0, -48.0, -7.0, -41.0];
    fill_curves(cr, red, (8.0, -41.0), &hat);
    line(cr, white, 3.6, LineCap::Round, -7.0, -41.4, 8.0, -41.4);
    fill_ellipse(cr, white, tip_x - 3.0, tip_y - 3.0, 6.0, 6.0);
}

/// The autumn theme's walkers (`DrawAnimal` on Windows). They are elves in everything but looks: they walk, stop,
/// wave, cross to the next screen and meet to hand over what they carry. A hedgehog carries an apple on its spines,
/// a squirrel an acorn in its paws. Which animal a walker is follows its clothes, so it stays the same animal.
/// Drawn in the elf's frame: feet at (0, 0), y up is negative, facing right.
fn animal(cr: &cairo::Context, e: &Elf) {
    let pi = std::f64::consts::PI;
    let walking = e.state == ElfState::Walk;
    let waving = e.state == ElfState::Wave;
    let p = if walking { e.walk_phase as f64 } else { 0.0 };
    let stand_phase = if e.state == ElfState::Stand { e.t as f64 * 1.5 } else { 0.0 };
    let wave_phase = if waving { e.t as f64 * 11.0 } else { 0.0 };
    let dark = rgb(52, 36, 26);
    let round = |col: Argb, width: f64, x1: f64, y1: f64, x2: f64, y2: f64| {
        line(cr, col, width, LineCap::Round, x1, y1, x2, y2);
    };
    // a filled shape of curves: the start point, then two control points and the end per curve
    let fill_curves = |col: Argb, start: (f64, f64), xy: &[f64]| {
        cr.new_path();
        cr.move_to(start.0, start.1);
        for c in xy.chunks_exact(6) {
            cr.curve_to(c[0], c[1], c[2], c[3], c[4], c[5]);
        }
        cr.close_path();
        set(cr, col);
        let _ = cr.fill();
    };
    fill_ellipse(cr, (50, 60, 40, 20), -13.0, -2.0, 26.0, 4.0);
    let carrying = e.carries_gift && !waving;

    if TUNICS.iter().position(|t| *t == e.tunic).unwrap_or(0) % 2 == 0 {
        // ---- hedgehog
        let (spine, spine_dark, fur) = (rgb(104, 74, 50), rgb(70, 48, 32), rgb(216, 182, 142));
        let bob = if walking { -p.sin().abs() } else { 0.0 };
        // sniffing the air when it "waves"
        let sniff = if waving { -4.0 - wave_phase.sin() * 2.0 } else { stand_phase.sin() * 0.6 };
        for (fx, phase) in [(-10.0, 0.0), (-4.0, pi), (5.0, pi), (11.0, 0.0)] {
            let to = fx + if walking { (p + phase).sin() * 3.0 } else { 0.0 };
            round(spine_dark, 3.6, fx, -5.0 + bob, to, 0.0);
        }
        cr.translate(0.0, bob);
        // the face: a tan cone ending in the nose
        fill_curves(
            fur,
            (4.0, -22.0),
            &[14.0, -22.0, 20.0, -14.0, 25.0, -7.0 + sniff, 22.0, -3.0, 14.0, -2.0, 4.0, -2.0],
        );
        // the spiny back
        fill_curves(
            spine,
            (-19.0, -2.0),
            &[-22.0, -22.0, -6.0, -29.0, 6.0, -24.0, 12.0, -20.0, 12.0, -8.0, 8.0, -2.0],
        );
        for i in 0..10 {
            let a = pi * (1.02 - 0.84 * i as f64 / 9.0);
            let (bx, by) = (-5.0 + a.cos() * 15.0, -11.0 - a.sin() * 15.0);
            round(spine_dark, 1.8, bx, by, bx + a.cos() * 6.0, by - a.sin() * 6.0);
        }
        for i in 0..6 {
            let a = pi * (0.9 - 0.7 * i as f64 / 5.0);
            let (bx, by) = (-5.0 + a.cos() * 8.0, -9.0 - a.sin() * 8.0);
            round(spine_dark, 1.4, bx, by, bx + a.cos() * 5.0, by - a.sin() * 5.0);
        }
        fill_ellipse(cr, spine, 6.0, -22.0, 6.0, 6.0); // ear
        fill_ellipse(cr, dark, 14.0, -15.0 + sniff * 0.4, 3.2, 3.2); // eye
        fill_ellipse(cr, dark, 22.5, -9.5 + sniff, 4.5, 4.5); // nose
        if carrying {
            // an apple stuck on the spines
            let apples = [rgb(206, 48, 44), rgb(196, 176, 52), rgb(222, 120, 40)];
            let which = GIFTS.iter().position(|g| *g == e.gift).unwrap_or(0) % 3;
            fill_ellipse(cr, apples[which], -11.0, -41.0, 13.0, 12.0);
            round(spine_dark, 1.6, -4.5, -41.0, -3.5, -45.0);
            fill_ellipse(cr, rgb(88, 150, 60), -3.5, -47.0, 6.0, 3.5);
            fill_ellipse(cr, (120, 255, 255, 255), -8.5, -39.0, 3.0, 3.0);
        }
    } else {
        // ---- squirrel
        let (rust, rust_dark, cream) = (rgb(192, 94, 46), rgb(150, 66, 30), rgb(242, 222, 190));
        let bob = if walking { -p.sin().abs() * 3.0 } else { 0.0 };
        let flick = if walking {
            (p * 0.5).sin() * 2.0
        } else {
            stand_phase.sin() * 1.5 + if waving { wave_phase.sin() * 3.0 } else { 0.0 }
        };
        cr.translate(0.0, bob);
        // the bushy tail, curling up behind the back
        fill_curves(
            rust,
            (-7.0, -5.0),
            &[
                -31.0,
                -6.0,
                -33.0 + flick,
                -38.0,
                -16.0 + flick,
                -47.0,
                -7.0 + flick,
                -51.0,
                -1.0 + flick,
                -41.0,
                -8.0,
                -35.0,
                -17.0,
                -28.0,
                -13.0,
                -15.0,
                -5.0,
                -9.0,
            ],
        );
        round(rust_dark, 1.4, -18.0 + flick * 0.6, -40.0, -22.0, -20.0);
        // hind leg and foot
        let step = if walking { p.sin() * 3.0 } else { 0.0 };
        fill_ellipse(cr, rust_dark, -7.0, -11.0, 15.0, 10.0);
        round(rust_dark, 4.0, 1.0, -3.0, 8.0 + step, -1.0);
        // body and belly
        fill_ellipse(cr, rust, -9.0, -30.0, 17.0, 27.0);
        fill_ellipse(cr, cream, -1.0, -25.0, 8.0, 19.0);
        // head, ear, eye, nose
        fill_ellipse(cr, rust, 0.0, -43.0, 16.0, 14.0);
        fill_polygon(cr, rust_dark, &[(3.0, -41.0), (5.0, -51.0), (10.0, -42.0)]);
        fill_ellipse(cr, cream, 9.0, -36.0, 7.0, 6.0);
        fill_ellipse(cr, dark, 9.5, -39.0, 3.0, 3.0);
        fill_ellipse(cr, dark, 15.0, -35.0, 2.6, 2.6);
        // the front paw: holding the acorn, waving, or swinging
        let (hand_x, hand_y) = if waving {
            (11.0, -37.0 + wave_phase.sin() * 3.0)
        } else if e.carries_gift {
            (11.0, -24.0)
        } else {
            (9.0 + if walking { p.sin() * 2.0 } else { 0.0 }, -20.0)
        };
        if carrying {
            let cup = rgb(104, 70, 34);
            fill_ellipse(cr, rgb(176, 128, 66), 9.0, -27.0, 8.0, 10.0);
            fill_ellipse(cr, cup, 8.5, -29.0, 9.0, 5.0);
            round(cup, 1.4, 13.0, -29.0, 13.0, -31.5);
        }
        round(rust_dark, 3.4, 3.0, -25.0, hand_x, hand_y);
    }
}

/// `JULE_ELF_SECONDS=N` (the Windows name `JULE_NISSE_SECONDS` works too): an elf arrives or leaves every N
/// seconds, for testing.
fn elf_seconds() -> Option<f32> {
    ["JULE_ELF_SECONDS", "JULE_NISSE_SECONDS"]
        .iter()
        .find_map(|k| std::env::var(k).ok()?.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.0)
}
