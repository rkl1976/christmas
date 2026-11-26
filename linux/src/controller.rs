//! The main loop: one overlay per monitor, the shared scene state (time, wind, weather), the frame timer, the
//! tray menu, the calendar window and its effects, the daily notices, the sound, and the test aids. Corresponds
//! to `src/Controller.cs`.
//!
//! Everything stops while nobody can see it: the session is locked, the screen saver runs, or the display is
//! off. Each reason is tracked on its own, so e.g. the screen saver ending in front of the lock screen does not
//! wake the overlay.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};
use x11rb::protocol::Event;

use crate::calendar::{Calendar, CalendarAction};
use crate::figures::{Figures, ScreenInfo};
use crate::folk;
use crate::layer::{Layer, PixelStore, Present, SceneState, SNOWMAN_MAX_STAGE};
use crate::rng::Rng;
use crate::session::SessionWatch;
use crate::snowman_build::SnowmanBuild;
use crate::settings::Settings;
use crate::sound::{AudioHub, Voice};
use crate::text;
use crate::tray::{TrayHandle, TrayMsg};
use crate::weather::Weather;
use crate::window_snow::WindowSnow;
use crate::x11::{Display, Monitor, OverlayWindow, Rect, Res};

#[derive(Default)]
pub struct Options {
    /// `--snapshot=DIR[,SEC]`: write each screen as a PNG after SEC seconds (default 5).
    pub snapshot: Option<(PathBuf, f32)>,
    /// `--santa`: send Santa right away.
    pub santa: bool,
    /// `--plow[=N]`: send the snow plow after N seconds (default 3).
    pub plow_after: Option<f32>,
    /// `--calendar`: open the calendar at start.
    pub calendar: bool,
    /// `--effect=N`: play a calendar effect at start (1 Santa, 2 storm, 3 elf party, 4 star shower, 5 all).
    pub effect: Option<i32>,
}

/// The sounds, made once and played when their figure is on its way.
struct Sounds {
    hub: AudioHub,
    jingles: Vec<Voice>,
    plow_engine: Voice,
    plow_scrape: Voice,
    plow_horn: Voice,
    /// Autumn's sounds: the geese's two moods (`GOOSE_MOODS`) and the leaf collector's suction.
    geese: Vec<Voice>,
    leaf_suction: Voice,
    /// De dødes dag: the butterflies' tune, and the donkey cart's hooves and wheel, the broom and the bray.
    butterfly_tune: Voice,
    cart_roll: Voice,
    broom_sweep: Voice,
    donkey_bray: Voice,
    /// Halloween: the witch's tune, and the wind the ghosts blow the leaves away with.
    witch_tune: Voice,
    ghost_wind: Voice,
    /// New Year: "Auld Lang Syne" to the rockets, and twelve strokes of a bell at midnight.
    auld_tune: Voice,
    midnight_bell: Voice,
    /// Carnival: the band truck's samba drums, and the Rhineland wagon's brass band (a recording).
    drum_tune: Voice,
    march_tune: Voice,
    /// This trip's tune (in autumn: the geese's mood).
    jingle: usize,
    horned_this_run: bool,
    /// Where Santa was last (x, scale, span), so his tune can fade out after he has gone.
    santa_last: (f32, f32, f32, f32),
}

struct Screen {
    layer: Layer,
    /// None when no window can be shown (no compositor); the scene still runs for `--snapshot`.
    window: Option<OverlayWindow>,
    /// False while a fullscreen app pauses this screen.
    mapped: bool,
}

pub struct Controller {
    display: Display,
    cfg: Settings,
    screens: Vec<Screen>,
    monitors: Vec<Monitor>,
    weather: Weather,
    rng: Rng,
    tray: Option<TrayHandle>,
    tray_rx: Receiver<TrayMsg>,
    snapshot: Option<(PathBuf, f32)>,
    figures: Figures,
    /// The snowman the children build in the Christmas theme (drawn by the primary screen; it lives here, so it
    /// survives the screens being rebuilt).
    build: SnowmanBuild,
    /// The stage today of what grows by the day (autumn's haystack), 1..=12 (checked every 30 seconds, for the day
    /// changing at midnight).
    snowman_stage: i32,
    day_timer: f32,
    last_day: NaiveDate,
    calendar: Option<Calendar>,
    sounds: Sounds,
    /// Snow on other windows' top edges, and when to look at the windows again.
    window_snow: WindowSnow,
    scan_timer: f32,
    /// A calendar storm lasts until this time.
    storm_until: f64,
    /// New Year's clock: the countdown to midnight and what is due when (`new_year.rs`).
    new_year: crate::new_year::NewYear,
    /// How many beats of carnival's drums have gone (`BEAT` seconds each): the walkers step in time with it on
    /// every screen, and the band on the truck plays to it, whether the sound is on or not (`advance_beat`).
    beat_count: f64,
    /// The snowman grew today: celebrate with sparks at this time.
    celebrate_at: Option<f64>,
    /// The weather is a storm now (in autumn the geese then always call over wind).
    storm_now: bool,
    /// A release build checks for updates 1 minute after start and then every 6 hours.
    update_timer: f32,
    /// A check running on a worker thread; `true` when asked for from the menu (then every outcome is reported).
    update_check: Option<(Receiver<crate::updater::Outcome>, bool)>,

    /// Seconds since start, as a double (see `SceneState::time`).
    time: f64,
    wind: f32,
    wind_target: f32,
    wind_timer: f32,
    snow_phase_a: f64,
    snow_phase_b: f64,

    can_overlay: bool,
    compositor_timer: f32,
    top_timer: f32,
    /// Fullscreen window and panels, checked once a second.
    desktop_timer: f32,
    fullscreen: Option<Rect>,
    /// Battery state, checked every 5 seconds.
    power_timer: f32,
    power_save: bool,

    session: SessionWatch,
    display_off: bool,
    next_dpms_check: Instant,
    asleep: bool,
    /// Monitors changed: rebuild after this many seconds (RandR sends bursts of events).
    rebuild_in: Option<f32>,
}

impl Controller {
    pub fn new(opts: Options) -> Res<Controller> {
        let display = Display::open()?;
        let mut cfg = Settings::load();
        crate::lang::select(&cfg.language);
        if cfg.first_run.is_none() {
            // the snowman is built over 12 days from the first start
            cfg.first_run = Some(crate::clock::today());
            cfg.save();
        }
        let mut rng = Rng::from_time();
        let can_overlay = display.can_overlay();
        let (tx, tray_rx) = channel();
        let today = crate::clock::today();
        let tray = crate::tray::spawn(
            tx,
            status_text(can_overlay),
            cfg.clone(),
            snowman_stage_for(&cfg, today),
            countdown_text(today),
            crate::autostart::enabled(),
        );
        let session = crate::session::spawn();
        let snow_phase_a = rng.f64() * 2.0 * std::f64::consts::PI;
        let snow_phase_b = rng.f64() * 2.0 * std::f64::consts::PI;
        let figures_seed = rng.next_u64();
        let build_seed = rng.next_u64();
        let (display_scale, window_snow_seed) = (display.dpi_scale, rng.next_u64());
        let mut c = Controller {
            display,
            cfg,
            screens: Vec::new(),
            monitors: Vec::new(),
            weather: Weather::new(),
            rng,
            tray,
            tray_rx,
            snapshot: opts.snapshot,
            figures: Figures::new(figures_seed),
            build: SnowmanBuild::new(build_seed),
            snowman_stage: 1,
            day_timer: 30.0,
            last_day: today,
            calendar: None,
            sounds: Sounds::new(),
            window_snow: WindowSnow::new(display_scale, window_snow_seed),
            scan_timer: 0.0,
            storm_until: -1.0,
            new_year: crate::new_year::NewYear::new(),
            beat_count: 0.0,
            celebrate_at: None,
            storm_now: false,
            // JULE_UPDATE_DELAY=N: the first check after N seconds (testing)
            update_timer: std::env::var("JULE_UPDATE_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(60.0),
            update_check: None,
            time: 0.0,
            wind: 0.0,
            wind_target: 0.0,
            wind_timer: 0.0,
            snow_phase_a,
            snow_phase_b,
            can_overlay,
            compositor_timer: 2.0,
            top_timer: 2.0,
            desktop_timer: 0.0,
            fullscreen: None,
            power_timer: 0.0,
            power_save: false,
            session,
            display_off: false,
            next_dpms_check: Instant::now(),
            asleep: false,
            rebuild_in: None,
        };
        c.update_power_save();
        // the "updated" notice takes the place of the start notice (Windows shows only the last balloon)
        let updated = c.announce_update();
        c.update_daily(!updated);
        c.warm_sounds();
        c.build_screens();
        c.figures.autumn = c.cfg.theme == 1;
        c.figures.muertos = c.cfg.theme == 2;
        c.figures.halloween = c.cfg.theme == 3;
        c.figures.nytaar = c.cfg.theme == 4;
        c.figures.karneval = c.cfg.theme == 5;
        c.figures.rhein = c.rhein();
        if opts.santa {
            let screens = c.screen_infos();
            c.figures.launch_santa(&screens);
        }
        if let Some(secs) = opts.plow_after {
            c.figures.send_plow_after(0.0, secs);
        }
        if opts.calendar {
            c.show_calendar();
        }
        if let Some(n) = opts.effect {
            c.trigger_effect(n);
        }
        Ok(c)
    }

    // ------------------------------------------------------------------ screens
    fn build_screens(&mut self) {
        for s in self.screens.drain(..) {
            if let Some(w) = &s.window {
                self.display.destroy_overlay(w);
            }
        }
        self.monitors = self.display.monitors();
        if !self.can_overlay && self.snapshot.is_none() {
            crate::log::error("no compositor (or no 32-bit visual): the overlay stays off until one runs");
            return;
        }
        for m in &self.monitors {
            if !self.cfg.all_screens && !m.primary {
                continue;
            }
            let seed = self.rng.next_u64();
            // shared memory only for a real window (a --snapshot run without a compositor has none)
            let shm = if self.can_overlay { self.display.shm_buffer(m.rect.w, m.rect.h) } else { None };
            let seg = shm.as_ref().map(|(_, seg)| *seg);
            let pixels: Box<dyn PixelStore> = match shm {
                Some((buf, _)) => Box::new(buf),
                None => Box::new(vec![0u32; m.rect.w as usize * m.rect.h as usize]),
            };
            let layer = match Layer::with_pixels(m.rect, m.primary, self.display.dpi_scale, seed, pixels) {
                Ok(l) => l,
                Err(e) => {
                    crate::log::error(&format!("screen {} {:?}: {e}", m.name, m.rect));
                    continue;
                }
            };
            let window = if self.can_overlay {
                match self.display.create_overlay(m.rect, seg) {
                    Ok(w) => Some(w),
                    Err(e) => {
                        crate::log::error(&format!("overlay for {}: {e}", m.name));
                        None
                    }
                }
            } else {
                None
            };
            crate::log::debug(&format!(
                "screen {} {}x{}+{}+{} primary {} window {} shm {}",
                m.name,
                m.rect.w,
                m.rect.h,
                m.rect.x,
                m.rect.y,
                m.primary,
                window.is_some(),
                seg.is_some()
            ));
            self.screens.push(Screen { layer, window, mapped: true });
        }
        // the elves need to know where they can walk on to, and the plow names its screens by index
        let rects: Vec<Rect> = self.screens.iter().map(|s| s.layer.rect).collect();
        for (i, s) in self.screens.iter_mut().enumerate() {
            s.layer.index = i;
            s.layer.neighbor_left = crate::figures::neighbor(&rects, i, -1.0).is_some();
            s.layer.neighbor_right = crate::figures::neighbor(&rects, i, 1.0).is_some();
        }
        self.figures.screens_changed();
        // the piles' sizes follow the desktop scale; X11 has one for all screens
        self.window_snow.set_scale(self.display.dpi_scale);
        self.update_desktop();
    }

    /// The screens as the figures in global coordinates see them.
    fn screen_infos(&self) -> Vec<ScreenInfo> {
        self.screens
            .iter()
            .map(|s| ScreenInfo {
                rect: s.layer.rect,
                s: s.layer.scale(),
                primary: s.layer.primary,
                heap_average: s.layer.heap_average(),
            })
            .collect()
    }

    // ------------------------------------------------------------------ daily events
    /// At start and when the day changes: a new calendar year, the snowman's step today, and a notice for the
    /// snowman growing or today's door (`UpdateDaily` on Windows).
    fn update_daily(&mut self, startup: bool) {
        let today = crate::clock::today();
        self.last_day = today;
        crate::calendar::sync_year(&mut self.cfg, today);
        self.snowman_stage = snowman_stage_for(&self.cfg, today);
        crate::log::debug(&format!("day {today}: snowman stage {}", self.snowman_stage));

        let mut notice: Option<(&str, String, bool)> = None;
        if self.snowman_stage > self.cfg.snowman_seen_stage {
            if self.cfg.snowman_seen_stage > 0 && self.cfg.snowman {
                let at = self.snowman_stage as usize - 1;
                // (only autumn's haystack grows by the day; the Christmas snowman is built while you watch)
                if self.cfg.theme == 1 {
                    notice =
                        Some((text::NOTICE_SCARECROW_TITLE, text::notice_scarecrow(text::SCARECROW_STEPS[at]), false));
                    self.celebrate_at = Some(self.time + 2.0);
                }
            }
            self.cfg.snowman_seen_stage = self.snowman_stage;
            self.cfg.save();
        }
        let day = today.day();
        if today.month() == 12
            && day <= 24
            && !self.cfg.opened_doors.contains(&(day as i32))
            && self.cfg.last_door_notice != Some(today)
        {
            notice = Some((text::NOTICE_DOOR_TITLE, text::notice_door(day), true));
            self.cfg.last_door_notice = Some(today);
            self.cfg.save();
        }
        if notice.is_none() && startup {
            notice = Some((text::NOTICE_STARTED_TITLE, text::NOTICE_STARTED.into(), false));
        }
        if let Some((title, body, opens_calendar)) = notice {
            self.session.notify(title, &body, opens_calendar);
        }
        self.update_tray();
    }

    /// How far the ofrenda of De dødes dag has come (1..=12): a step for every hour the app has run with that theme.
    /// Finished, it stands until another theme is chosen; choosing the theme again starts a new altar.
    fn ofrenda_stage(&self) -> i32 {
        ((self.cfg.ofrenda_seconds / OFRENDA_STEP_TIME) as i32 + 1).clamp(1, crate::layer::SNOWMAN_MAX_STAGE)
    }

    /// The ofrenda grows while the app runs with the theme on. Each new step gets sparkles; the last one also a
    /// notice. The time is kept in the settings (at each step and once a minute), so a restart carries on.
    /// A theme that is only on for a couple of days builds its form of the snowman a stage for every hour the app
    /// runs: the ofrenda of De dødes dag (2), and Halloween's jack-o'-lanterns (3), which share its clock.
    fn grows_by_hour(&self) -> bool {
        // (and New Year's clock, 4, is decorated the same way, and carnival's float, 5, built)
        (2..=5).contains(&self.cfg.theme)
    }

    /// The Karneval theme in its Rhineland form (setting `Carnival`, or by language and region).
    fn rhein(&self) -> bool {
        self.cfg.theme == 5 && folk::carnival_chosen(self.cfg.carnival) == folk::CARNIVAL_RHEIN
    }

    /// Moves carnival's beat on. With no music it is a clock's. While the Rhineland wagon's band is heard, the beat
    /// is the band's own: where the recording is right now, looked up among its measured beats (a real band's
    /// tempo wanders, so a clock would be half a step off within half a minute). The walkers are eased over to it,
    /// never jumped: the difference (counted in pairs of beats, so nobody changes foot) is closed over a second.
    fn advance_beat(&mut self, dt: f32) {
        let dt = dt as f64;
        let mut step = dt / BEAT;
        if self.rhein() {
            let s = &self.sounds;
            if let Some(music) = s.hub.position(s.march_tune).and_then(crate::sound::rhein_beat) {
                let mut off = (music - self.beat_count).rem_euclid(2.0);
                if off >= 1.0 {
                    off -= 2.0;
                }
                step += off * (dt * 4.0).min(1.0);
            }
        }
        self.beat_count += step;
    }

    fn update_ofrenda(&mut self, dt: f32) {
        let max = crate::layer::SNOWMAN_MAX_STAGE;
        if !self.grows_by_hour() || !self.cfg.snowman || self.ofrenda_stage() >= max {
            return;
        }
        let (before, was) = (self.ofrenda_stage(), self.cfg.ofrenda_seconds);
        self.cfg.ofrenda_seconds += dt as f64 * ofrenda_speed();
        let stage = self.ofrenda_stage();
        if stage != before {
            crate::log::debug(&format!("ofrenda: step {stage} of {max}"));
            self.celebrate_at = Some(self.time + 0.5);
            if stage == max && self.cfg.theme == 5 {
                if self.rhein() {
                    self.session.notify(text::NOTICE_PRUNKWAGEN_TITLE, text::NOTICE_PRUNKWAGEN, false);
                } else {
                    let last = text::FLOAT_STEPS[text::FLOAT_STEPS.len() - 1];
                    self.session.notify(text::NOTICE_FLOAT_TITLE, &text::notice_float(last), false);
                }
            } else if stage == max && self.cfg.theme == 4 {
                self.session.notify(text::NOTICE_CLOCK_TITLE, &text::notice_clock(), false);
            } else if stage == max && self.cfg.theme == 3 {
                self.session.notify(text::NOTICE_PUMPKINS_TITLE, &text::notice_pumpkins(), false);
            } else if stage == max {
                self.session.notify(text::NOTICE_OFRENDA_TITLE, &text::notice_ofrenda(), false);
            }
            self.update_tray();
        }
        if stage != before || (was / 60.0) as i64 != (self.cfg.ofrenda_seconds / 60.0) as i64 {
            self.cfg.save();
        }
    }

    /// Shows the current settings, the snowman's stage and the countdown in the tray menu.
    fn update_tray(&self) {
        let Some(t) = &self.tray else { return };
        let stage = if self.grows_by_hour() { self.ofrenda_stage() } else { self.snowman_stage };
        // (at New Year the line counts down to midnight)
        let countdown =
            if self.cfg.theme == 4 { self.new_year.countdown.clone() } else { countdown_text(self.last_day) };
        let cfg = self.cfg.clone();
        let autostart = crate::autostart::enabled();
        let _ = t.update(move |tray: &mut crate::tray::AppTray| {
            tray.cfg = cfg;
            tray.snowman_stage = stage;
            tray.countdown = countdown;
            tray.autostart = autostart;
        });
    }

    fn handle_tray(&mut self, msg: TrayMsg) {
        match msg {
            TrayMsg::Quit => {}
            TrayMsg::CheckUpdate => self.check_for_update(true),
            TrayMsg::Calendar => self.show_calendar(),
            // (in autumn the geese fly in his place)
            TrayMsg::Santa => {
                let screens = self.screen_infos();
                self.figures.autumn = self.cfg.theme == 1;
                self.figures.muertos = self.cfg.theme == 2;
                self.figures.halloween = self.cfg.theme == 3;
                self.figures.nytaar = self.cfg.theme == 4;
                self.figures.karneval = self.cfg.theme == 5;
                self.figures.rhein = self.rhein();
                self.figures.launch_santa(&screens);
            }
            TrayMsg::Plow => {
                let screens = self.screen_infos();
                self.figures.halloween = self.cfg.theme == 3;
                self.figures.nytaar = self.cfg.theme == 4;
                self.figures.karneval = self.cfg.theme == 5;
                self.figures.send_plow(&screens);
            }
            TrayMsg::Settings(m) => {
                let old = self.cfg.take_menu_fields(&m);
                self.cfg.save();
                if old.language != self.cfg.language {
                    // (the countdown is made here and handed to the menu as a finished text)
                    crate::lang::select(&self.cfg.language);
                    self.update_tray();
                }
                crate::log::debug("settings changed in the menu");
                // (another carnival is as much of a change as another theme: the float, the garland and what falls
                // are all kept as pictures, so the screens are built anew)
                let carnival = self.cfg.theme == 5 && old.carnival != self.cfg.carnival;
                if old.all_screens != self.cfg.all_screens || old.theme != self.cfg.theme || carnival {
                    // (a new theme starts from empty screens: nothing of the old one is left falling or lying)
                    if old.theme != self.cfg.theme {
                        self.finish_runs();
                        // the ofrenda is taken down: a new one is built from its first step the next time De
                        // dødes dag is chosen
                        self.cfg.ofrenda_seconds = 0.0;
                        self.celebrate_at = None;
                        self.cfg.save();
                        self.warm_sounds();
                        self.update_tray();
                    }
                    if carnival {
                        self.warm_sounds();
                    }
                    self.build_screens();
                }
                if old.power_save != self.cfg.power_save {
                    self.update_power_save();
                }
                if (!old.sound && self.cfg.sound) || (!old.plow_sound && self.cfg.plow_sound) {
                    self.warm_sounds();
                }
                self.update_tray();
            }
            TrayMsg::Autostart(on) => {
                crate::autostart::set(on);
                self.update_tray();
            }
        }
    }

    // ------------------------------------------------------------------ automatic update
    /// After an update: "Jule-hygge er opdateret" once, with the new version.
    /// Returns true when it showed the notice.
    fn announce_update(&mut self) -> bool {
        if !crate::updater::is_release_build() {
            return false;
        }
        let now = crate::updater::display(crate::updater::current());
        if self.cfg.last_version == now {
            return false;
        }
        let shown = !self.cfg.last_version.is_empty();
        if shown {
            self.session.notify(text::NOTICE_UPDATED_TITLE, &text::notice_updated(&now), false);
        }
        self.cfg.last_version = now;
        self.cfg.save();
        shown
    }

    /// Asks GitHub for a newer release on a worker thread (`manual`: asked for from the menu).
    fn check_for_update(&mut self, manual: bool) {
        if self.update_check.is_some() || !crate::updater::is_release_build() {
            return;
        }
        let (tx, rx) = channel();
        let started = std::thread::Builder::new().name("update".into()).spawn(move || {
            let _ = tx.send(crate::updater::check_and_download());
        });
        match started {
            Ok(_) => self.update_check = Some((rx, manual)),
            Err(e) => crate::log::error(&format!("update: {e}")),
        }
    }

    /// The check's outcome, when it has finished: a newer version is put in place, and the app then quits with
    /// `RESTART_EXIT_CODE` (the supervisor starts the new binary). Returns that exit code.
    fn poll_update(&mut self) -> Option<i32> {
        let (rx, manual) = self.update_check.as_ref()?;
        let manual = *manual;
        let outcome = match rx.try_recv() {
            Ok(o) => o,
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                crate::updater::Outcome::Failed("the update check stopped".into())
            }
        };
        self.update_check = None;
        let current = crate::updater::display(crate::updater::current());
        match outcome {
            crate::updater::Outcome::UpToDate => {
                if manual {
                    self.session.notify(text::NOTICE_UPDATE_TITLE, &text::notice_up_to_date(&current), false);
                }
                None
            }
            crate::updater::Outcome::Failed(e) => {
                crate::log::error(&format!("update failed: {e}"));
                if manual {
                    self.session.notify(text::NOTICE_UPDATE_TITLE, &text::notice_update_failed(&e), false);
                }
                None
            }
            crate::updater::Outcome::Ready { version, file } => {
                let v = crate::updater::display(version);
                if let Err(e) = crate::updater::install(&file) {
                    crate::log::error(&format!("update: {e}"));
                    if manual {
                        self.session.notify(text::NOTICE_UPDATE_TITLE, &text::notice_install_failed(&v, &e), false);
                    }
                    return None;
                }
                crate::log::debug(&format!("update: installed {v}, restarting"));
                Some(crate::updater::RESTART_EXIT_CODE)
            }
        }
    }

    // ------------------------------------------------------------------ calendar and effects
    /// Opens the calendar, or brings it to the front if it is open.
    fn show_calendar(&mut self) {
        if let Some(c) = &self.calendar {
            c.raise(&self.display);
            return;
        }
        let on = self.monitors.iter().find(|m| m.primary).or(self.monitors.first()).map(|m| m.rect);
        match Calendar::open(&self.display, &self.cfg, on.unwrap_or(Rect::new(0, 0, 1920, 1080))) {
            Ok(c) => self.calendar = Some(c),
            Err(e) => crate::log::error(&format!("calendar: {e}")),
        }
    }

    /// What a calendar door does: 1 Santa, 2 a 25-second snowstorm, 3 the elves wave, 4 a star shower, 5 all but
    /// the storm.
    fn trigger_effect(&mut self, effect: i32) {
        crate::log::debug(&format!("effect {effect}"));
        let screens = self.screen_infos();
        match effect {
            1 => self.figures.launch_santa(&screens),
            2 => self.storm_until = self.time + 25.0,
            3 => self.elf_party(),
            4 => self.figures.star_shower(&screens),
            5 => {
                self.figures.launch_santa(&screens);
                self.figures.star_shower(&screens);
                self.elf_party();
            }
            _ => {}
        }
    }

    /// New Year: the countdown in the menu, the last ten seconds written in sparks over the clock, and at
    /// midnight about a minute of the big effect: rockets one after the other over every screen (unless they are
    /// switched off), a burst of confetti, the guests jump, and the new year's number in sparks. With the sound on,
    /// twelve strokes of a bell first, and the tune after them. (`UpdateNewYear` on Windows.)
    fn update_new_year(&mut self) {
        if self.cfg.theme != 4 {
            self.new_year.rest();
            return;
        }
        if self.new_year.refresh_countdown(self.time) {
            self.update_tray();
        }
        let due = self.new_year.update(self.time);
        if due.is_empty() {
            return;
        }
        let screens = self.screen_infos();
        let year = self.new_year.year(self.time);
        for moment in due {
            match moment {
                crate::new_year::Moment::Count(second) => {
                    // over the clock, or over the middle of the primary screen if the clock is switched off
                    let Some((i, sc)) = screens.iter().enumerate().find(|(_, s)| s.primary) else { continue };
                    let at = match self.screens[i].layer.snowman_pos() {
                        Some((x, y)) => (x, y - 178.0 * sc.s),
                        None => (sc.rect.x as f32 + sc.rect.w as f32 / 2.0, sc.rect.y as f32 + sc.rect.h as f32 * 0.45),
                    };
                    self.figures.write_in_sparks(second, at, 60.0 * sc.s, 0.95, sc.s);
                }
                crate::new_year::Moment::Midnight => {
                    crate::log::debug(&format!("midnight: {year}"));
                    self.storm_until = self.time + 35.0;
                    self.write_year(&screens, year);
                    let muted = self.cfg.pause_fullscreen && self.fullscreen.is_some();
                    if self.cfg.sound && !muted {
                        let volume = [0.15, 0.35, 0.6][self.cfg.sound_level.clamp(0, 2) as usize];
                        let s = &mut self.sounds;
                        s.hub.play_once(s.midnight_bell, volume, 0.0);
                        self.new_year.tune_hold_until = self.time + 16.5;
                    }
                    if self.cfg.santa {
                        self.figures.nytaar = true;
                        self.figures.launch_volley(&screens, 46.0, 0.5);
                    }
                }
                crate::new_year::Moment::Jump => self.elf_party(),
                crate::new_year::Moment::YearAgain => self.write_year(&screens, year),
            }
        }
    }

    /// The new year's number, large, in the top third of every screen.
    fn write_year(&mut self, screens: &[ScreenInfo], year: i32) {
        for sc in screens {
            let (b, s) = (sc.rect, sc.s);
            let at = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 * 0.3);
            self.figures.write_in_sparks(year, at, (150.0 * s).min(b.w as f32 / 7.0), 9.0, s);
        }
    }

    fn elf_party(&mut self) {
        for s in &mut self.screens {
            s.layer.elf_wave(6.0);
        }
    }

    // ------------------------------------------------------------------ sound
    /// Synthesizes the switched-on sounds in the background ahead of time, so they are ready when needed.
    fn warm_sounds(&mut self) {
        let s = &self.sounds;
        let autumn = self.cfg.theme == 1;
        let muertos = self.cfg.theme == 2;
        if self.cfg.sound {
            if muertos {
                s.hub.warm(s.butterfly_tune);
            } else if self.cfg.theme == 5 {
                s.hub.warm(if self.rhein() { s.march_tune } else { s.drum_tune });
            } else if self.cfg.theme == 4 {
                s.hub.warm(s.auld_tune);
                s.hub.warm(s.midnight_bell);
            } else if self.cfg.theme == 3 {
                s.hub.warm(s.witch_tune);
            } else {
                for j in if autumn { &s.geese } else { &s.jingles } {
                    s.hub.warm(*j);
                }
            }
        }
        if self.cfg.plow_sound {
            if muertos {
                for v in [s.cart_roll, s.broom_sweep, s.donkey_bray] {
                    s.hub.warm(v);
                }
            } else if self.cfg.theme == 3 {
                for v in [s.broom_sweep, s.ghost_wind] {
                    s.hub.warm(v);
                }
            } else if self.cfg.theme == 5 {
                s.hub.warm(s.broom_sweep);
            } else if self.cfg.theme == 4 {
                for v in [s.plow_engine, s.leaf_suction] {
                    s.hub.warm(v);
                }
            } else {
                for v in [s.plow_engine, if autumn { s.leaf_suction } else { s.plow_scrape }, s.plow_horn] {
                    s.hub.warm(v);
                }
            }
        }
    }

    /// Pans each sound with what makes it (Santa, the plow) across all screens and fades it at the edges. A
    /// fullscreen app that pauses the snow mutes it too.
    fn update_sound(&mut self, dt: f32) {
        let volume = [0.15, 0.35, 0.6][self.cfg.sound_level.clamp(0, 2) as usize];
        let muted = self.cfg.pause_fullscreen && self.fullscreen.is_some();
        let autumn = self.cfg.theme == 1;
        let muertos = self.cfg.theme == 2;
        let halloween = self.cfg.theme == 3;
        let nytaar = self.cfg.theme == 4;
        let karneval = self.cfg.theme == 5;
        let rhein = self.rhein();
        let ghosts_blow = self.figures.ghosts_blow;
        let christmas = !autumn && !muertos && !halloween && !nytaar && !karneval;
        let tune_held = self.time < self.new_year.tune_hold_until;
        let storm = self.storm_now;
        let s = &mut self.sounds;
        if self.figures.take_new_trip() {
            if muertos {
                // the butterflies' tune, at the trip's pace
                s.hub.set_rate(s.butterfly_tune, self.figures.santa_pace);
            } else if halloween {
                // the witch's tune
                s.hub.set_rate(s.witch_tune, self.figures.santa_pace);
            } else if nytaar {
                // "Auld Lang Syne" to the rockets
                s.hub.set_rate(s.auld_tune, self.figures.santa_pace);
            } else if karneval {
                // carnival's music keeps its own tempo, which is the beat the walkers step to: never faster or
                // slower
                s.hub.set_rate(s.drum_tune, 1.0);
                s.hub.set_rate(s.march_tune, 1.0);
            } else if autumn {
                // the geese: alone two trips out of three, otherwise in wind; in a storm always the one with wind
                s.jingle = usize::from(storm || self.rng.f() < 1.0 / 3.0);
                s.hub.set_rate(s.geese[s.jingle], self.figures.santa_pace);
            } else {
                // each trip picks one of the tunes, played at the trip's pace
                s.jingle = self.rng.range(0, s.jingles.len() as i32) as usize;
                s.hub.set_rate(s.jingles[s.jingle], self.figures.santa_pace);
            }
        }
        if let (Some(santa), Some((min, max))) = (&self.figures.santa, self.figures.santa_span()) {
            s.santa_last = (santa.x, santa.scale, min, max);
        }
        {
            let (x, scale, min, max) = s.santa_last;
            let margin = 330.0 * scale;
            let presence = ((x - (min - margin)).min((max + margin) - x) / margin).clamp(0.0, 1.0);
            let pan = (x - min) / (max - min).max(1.0) * 2.0 - 1.0;
            let on = self.figures.santa.is_some() && self.cfg.sound && !muted;
            // only this trip's tune plays; any other one still fading out keeps being updated until it is silent
            for i in 0..s.jingles.len() {
                let v = s.jingles[i];
                if (self.cfg.sound && christmas && i == s.jingle) || s.hub.playing(v) {
                    let level = if on && christmas && i == s.jingle { presence * volume } else { 0.0 };
                    s.hub.update(v, level, pan, dt);
                }
            }
            // in autumn the geese call instead, quieter when they pass far off
            let depth = self.figures.flyer_depth.min(1.0);
            for i in 0..s.geese.len() {
                let v = s.geese[i];
                if (self.cfg.sound && autumn && i == s.jingle) || s.hub.playing(v) {
                    let level = if on && autumn && i == s.jingle { presence * depth * volume } else { 0.0 };
                    s.hub.update(v, level, pan, dt);
                }
            }
            // the Day of the Dead's butterflies have their own tune (guitar and marimba)
            if (self.cfg.sound && muertos) || s.hub.playing(s.butterfly_tune) {
                let level = if on && muertos { presence * depth * volume } else { 0.0 };
                s.hub.update(s.butterfly_tune, level, pan, dt);
            }
            // and Halloween's witch hers
            if (self.cfg.sound && halloween) || s.hub.playing(s.witch_tune) {
                let level = if on && halloween { presence * depth * volume } else { 0.0 };
                s.hub.update(s.witch_tune, level, pan, dt);
            }
            // New Year's rockets go up to "Auld Lang Syne" (at midnight the tune waits for the bell's twelve strokes)
            if (self.cfg.sound && nytaar) || s.hub.playing(s.auld_tune) {
                let level = if on && nytaar && !tune_held { presence * volume } else { 0.0 };
                s.hub.update(s.auld_tune, level, 0.0, dt);
            }
            // carnival's truck has its drums, and the Rhineland's wagon a brass band (a recording)
            if (self.cfg.sound && karneval && !rhein) || s.hub.playing(s.drum_tune) {
                let level = if on && karneval && !rhein { presence * volume } else { 0.0 };
                s.hub.update(s.drum_tune, level, pan, dt);
            }
            if (self.cfg.sound && rhein) || s.hub.playing(s.march_tune) {
                let level = if on && rhein { presence * volume } else { 0.0 };
                s.hub.update(s.march_tune, level, pan, dt);
            }
        }
        let plow = &self.figures.plow;
        if self.cfg.plow_sound
            || s.hub.playing(s.plow_engine)
            || s.hub.playing(s.plow_scrape)
            || s.hub.playing(s.leaf_suction)
            || s.hub.playing(s.cart_roll)
            || s.hub.playing(s.broom_sweep)
            || s.hub.playing(s.ghost_wind)
        {
            let (min, max) = self.figures.plow_span();
            let margin = 300.0;
            let presence = ((plow.gx - (min - margin)).min((max + margin) - plow.gx) / margin).clamp(0.0, 1.0);
            let pan = (plow.gx - min) / (max - min).max(1.0) * 2.0 - 1.0;
            let on = plow.active && self.cfg.plow_sound && !muted;
            // (New Year's street sweeper is a small machine: a quieter engine, and the leaf collector's suction)
            let engine =
                if on && !muertos && !halloween && !karneval { presence * volume * if nytaar { 0.4 } else { 0.8 } } else { 0.0 };
            s.hub.update(s.plow_engine, engine, pan, dt);
            // scraping gets louder with the amount of snow in front of the blade
            let load = (plow.pushed / 2000.0).sqrt().min(1.0);
            let scrape = if on && christmas { presence * volume * (0.15 + 0.85 * load) } else { 0.0 };
            s.hub.update(s.plow_scrape, scrape, pan, dt);
            // in autumn the leaf collector's suction takes the scraping's place
            if autumn || nytaar || s.hub.playing(s.leaf_suction) {
                let suction = if on && (autumn || nytaar) { presence * volume * (0.55 + 0.45 * load) } else { 0.0 };
                s.hub.update(s.leaf_suction, suction, pan, dt);
            }
            // the donkey cart: hooves and wheel all the way, the broom louder with more petals in front of it
            // (Halloween's witch has the same broom; the ghosts a hollow wind of their own)
            // (carnival's street sweepers have brooms too)
            let sweeps = muertos || (halloween && !ghosts_blow) || karneval;
            if sweeps || s.hub.playing(s.cart_roll) || s.hub.playing(s.broom_sweep) {
                let roll = if on && muertos { presence * volume * 0.9 } else { 0.0 };
                s.hub.update(s.cart_roll, roll, pan, dt);
                let sweep = if on && sweeps { presence * volume * (0.25 + 0.75 * load) * 0.7 } else { 0.0 };
                s.hub.update(s.broom_sweep, sweep, pan, dt);
            }
            if (halloween && ghosts_blow) || s.hub.playing(s.ghost_wind) {
                let wind = if on && halloween && ghosts_blow { presence * volume * (0.5 + 0.5 * load) } else { 0.0 };
                s.hub.update(s.ghost_wind, wind, pan, dt);
            }
            if on && !s.horned_this_run && presence > 0.6 {
                s.horned_this_run = true;
                // (the donkey brays where the plow honks; Halloween has nothing there: a truck's honk does not
                // belong with a witch or ghosts)
                // (nor has New Year's sweeper: it comes the morning after; nor have carnival's)
                if !halloween && !nytaar && !karneval {
                    let horn = if muertos { s.donkey_bray } else { s.plow_horn };
                    s.hub.play_once(horn, volume, pan);
                }
            }
        }
        if !plow.active {
            s.horned_this_run = false;
        }
        s.hub.idle_check(self.time);
    }

    fn handle_event(&mut self, e: Event) {
        if let Some(cal) = self.calendar.as_mut() {
            if event_window(&e) == Some(cal.window()) {
                match cal.handle(&mut self.display, &e, &mut self.cfg) {
                    CalendarAction::Closed => {
                        if let Some(c) = self.calendar.take() {
                            c.close(&self.display);
                        }
                    }
                    CalendarAction::Effect(n) => self.trigger_effect(n),
                    CalendarAction::None => {}
                }
                return;
            }
        }
        match e {
            Event::Expose(x) => {
                for s in &mut self.screens {
                    if s.window.as_ref().map(|w| w.win) == Some(x.window) {
                        s.layer.invalidate();
                    }
                }
            }
            Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => {
                self.rebuild_in = Some(1.0);
            }
            Event::Error(err) => crate::log::debug(&format!("X error: {err:?}")),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ scene state
    /// 0 light ... 2 storm, gliding. Setting 3 (changing weather) lets the weather choose.
    fn snow_amount(&mut self) -> f32 {
        if self.time < self.storm_until {
            2.0
        } else if self.cfg.snow_level == 3 {
            self.weather.amount() as f32
        } else {
            self.cfg.snow_level as f32
        }
    }

    /// Snowfall drifts between ~60% and ~140% of the chosen amount (two slow waves of about 3 and 7 minutes).
    fn snow_intensity(&self) -> f32 {
        if self.time < self.storm_until {
            return 1.0;
        }
        let t = self.time;
        let tau = 2.0 * std::f64::consts::PI;
        (1.0 + 0.25 * (t * tau / 190.0 + self.snow_phase_a).sin() + 0.15 * (t * tau / 430.0 + self.snow_phase_b).sin())
            as f32
    }

    fn update_wind(&mut self, dt: f32, level: i32) {
        self.wind_timer -= dt;
        if self.wind_timer <= 0.0 {
            self.wind_timer = 3.0 + self.rng.f() * 6.0;
            let storm = level == 2;
            self.wind_target =
                (self.rng.f() * 2.0 - 1.0) * if storm { 140.0 } else { 30.0 } + if storm { 60.0 } else { 0.0 };
        }
        self.wind += (self.wind_target - self.wind) * (dt * 0.5).min(1.0);
    }

    /// Power-save: always (1), or only while a laptop runs on battery (2, the default).
    fn update_power_save(&mut self) {
        let on = self.cfg.power_save == 1 || (self.cfg.power_save == 2 && crate::session::on_battery());
        if on != self.power_save {
            crate::log::debug(&format!("power-save {}", if on { "on" } else { "off" }));
            self.power_save = on;
        }
    }

    /// Where each screen's snow lies (panels can be moved, hidden or resized) and whether a fullscreen app is
    /// active. Always detected: screens either pause or move their ground to the screen bottom.
    fn update_desktop(&mut self) {
        let fs = self.display.fullscreen_window();
        if fs != self.fullscreen {
            crate::log::debug(&format!("fullscreen window: {fs:?}"));
            self.fullscreen = fs;
        }
        let rects: Vec<Rect> = self.screens.iter().map(|s| s.layer.rect).collect();
        for (s, top) in self.screens.iter_mut().zip(self.display.panel_tops(&rects)) {
            s.layer.set_panel_top(top);
        }
    }

    // ------------------------------------------------------------------ sleep
    /// True while nobody can see the overlay. DPMS is asked every 2 seconds; lock and screen saver come from the
    /// session thread.
    fn check_asleep(&mut self) -> bool {
        let now = Instant::now();
        if now >= self.next_dpms_check {
            self.next_dpms_check = now + Duration::from_secs(2);
            let off = self.display.display_off();
            if off != self.display_off {
                crate::log::debug(&format!("display {}", if off { "off" } else { "on" }));
                self.display_off = off;
            }
        }
        let (locked, saver) = (self.session.locked(), self.session.screen_saver());
        let asleep = locked || saver || self.display_off;
        if asleep != self.asleep {
            self.asleep = asleep;
            if asleep {
                self.finish_runs();
                crate::log::debug(&format!("asleep (locked={locked} saver={saver} display_off={})", self.display_off));
            } else {
                crate::log::debug("awake");
            }
        }
        asleep
    }

    /// Ends Santa's trip and the plow run, as if they had arrived, when the overlay stops (lock, screen saver,
    /// display off) or frames stalled (e.g. after suspend): otherwise they stay "on the way" while nothing moves.
    fn finish_runs(&mut self) {
        self.figures.finish_runs();
        self.window_snow.finish();
        self.sounds.hub.stop_all();
    }

    /// The calm scene runs at the chosen frame rate (power-save caps it at 12); only fast things get more frames.
    /// As on Windows, Santa and the plow are boosted to 30 fps (20 in power-save mode), not 60. JULE_FPS
    /// overrides it.
    fn desired_fps(&self) -> f64 {
        if let Some(f) = std::env::var("JULE_FPS").ok().and_then(|v| v.trim().parse::<f64>().ok()) {
            if f > 0.0 {
                return f;
            }
        }
        let fr = self.cfg.frame_rate as f64;
        let calm = if self.power_save { fr.min(12.0) } else { fr };
        let busy = self.figures.busy() || self.window_snow.busy() || self.screens.iter().any(|s| s.layer.is_busy());
        if busy {
            calm.max(if self.power_save { 20.0 } else { 30.0 })
        } else {
            calm
        }
    }

    // ------------------------------------------------------------------ main loop
    /// Runs until the app quits; returns the exit code (`updater::RESTART_EXIT_CODE` after an update).
    pub fn run(&mut self) -> Res<i32> {
        let mut last = Instant::now();
        let mut next_frame = Instant::now();
        let mut stats = Stats::new();
        loop {
            for e in self.display.poll_events()? {
                self.handle_event(e);
            }
            while let Ok(msg) = self.tray_rx.try_recv() {
                if let TrayMsg::Quit = msg {
                    self.shutdown();
                    return Ok(0);
                }
                self.handle_tray(msg);
            }
            if self.session.calendar_clicked() {
                self.show_calendar();
            }
            if let Some(code) = self.poll_update() {
                self.shutdown();
                return Ok(code);
            }

            if self.check_asleep() {
                // Keep handling events and the tray, but draw nothing (and do not raise the windows above a
                // lock screen).
                std::thread::sleep(Duration::from_millis(250));
                last = Instant::now();
                next_frame = last;
                continue;
            }

            let now = Instant::now();
            let gap = (now - last).as_secs_f32();
            // frames stopped for a while (e.g. suspend, or a screen saver we got no notice of)
            if gap > 2.0 {
                crate::log::debug(&format!("frames stalled for {gap:.1} s"));
                self.finish_runs();
            }
            let dt = gap.min(0.1);
            last = now;
            self.time += dt as f64;
            self.advance_beat(dt);
            self.timers(dt);
            self.update_ofrenda(dt);
            self.update_new_year();
            let screens = self.screen_infos();
            // The autumn theme: every figure has its autumn form (`SceneState::autumn`), and so have the sounds
            // (`update_sound`).
            let autumn = self.cfg.theme == 1;
            let muertos = self.cfg.theme == 2;
            let halloween = self.cfg.theme == 3;
            let nytaar = self.cfg.theme == 4;
            let karneval = self.cfg.theme == 5;
            self.figures.autumn = autumn;
            self.figures.muertos = muertos;
            self.figures.halloween = halloween;
            self.figures.nytaar = nytaar;
            self.figures.karneval = karneval;
            self.figures.rhein = self.rhein();
            self.figures.update(dt, self.time, &screens, self.cfg.santa, self.cfg.plow);
            // In the Christmas theme children build the snowman; autumn's haystack grows by the day.
            let children = self.cfg.snowman && !autumn && !muertos && !halloween && !nytaar && !karneval;
            let (clock, clock_key) = if nytaar { self.new_year.hands(self.time) } else { ((0.0, 0.0, None), 0) };
            let gather = nytaar && self.new_year.gather(self.time);
            if children {
                self.update_build(dt);
            }

            let amount = self.snow_amount();
            let level = if amount > 1.5 {
                2
            } else if amount > 0.5 {
                1
            } else {
                0
            };
            self.storm_now = level == 2;
            self.update_wind(dt, level);
            let bottom = self.screens.iter().map(|s| s.layer.rect.bottom()).max().unwrap_or(0) as f32;
            let melt = !(self.cfg.snow || self.time < self.storm_until) || !self.cfg.window_snow;
            self.window_snow.update(dt, melt, level == 2, self.wind, bottom);
            let st = SceneState {
                time: self.time,
                wind: self.wind,
                snow_amount: amount,
                snow_level: level,
                snow_on: self.cfg.snow || self.time < self.storm_until,
                snow_intensity: self.snow_intensity(),
                snow_ground: self.cfg.snow_ground,
                fullscreen: self.fullscreen,
                pause_fullscreen: self.cfg.pause_fullscreen,
                power_save: self.power_save,
                lights: self.cfg.lights,
                elves: self.cfg.elves,
                elf_count: self.cfg.elf_count,
                snowman_stage: if !self.cfg.snowman {
                    0
                } else if muertos || halloween || nytaar || karneval {
                    self.ofrenda_stage()
                } else if autumn {
                    self.snowman_stage
                } else {
                    0
                },
                build: children.then_some(&self.build),
                // (New Year's flyer is a volley of rockets: no figure is drawn for it)
                santa: if nytaar { None } else { self.figures.santa.as_ref() },
                sparks: &self.figures.sparks,
                plow: Some(&self.figures.plow),
                window_snow: self.cfg.window_snow,
                ledges: &self.window_snow.ledges,
                clumps: &self.window_snow.clumps,
                ledge_u: self.display.dpi_scale,
                autumn,
                muertos,
                halloween,
                ghosts_blow: self.figures.ghosts_blow,
                folk: folk::chosen(self.cfg.folk),
                kite: self.cfg.kite,
                nytaar,
                rockets: &self.figures.rockets,
                embers: &self.figures.embers,
                clock,
                clock_key,
                gather,
                carnival: if karneval { folk::carnival_chosen(self.cfg.carnival) } else { 0 },
                beat_count: self.beat_count,
            };
            let busy = Instant::now();
            frame_all(&mut self.display, &mut self.screens, dt, &st);
            self.after_frame();
            if let Some(cal) = self.calendar.as_mut() {
                cal.tick(&mut self.display, dt, &self.cfg);
            }
            self.update_sound(dt);
            self.display.flush()?;
            stats.frame(busy.elapsed(), self.desired_fps(), self.screens.iter().map(|s| s.layer.flake_count()).sum());

            if let Some((dir, at)) = &self.snapshot {
                if self.time >= *at as f64 {
                    let dir = dir.clone();
                    self.snapshot = None;
                    self.write_snapshots(&dir);
                }
            }

            // Sleep until the next frame; after a stall, start counting again from now.
            let period = Duration::from_secs_f64(1.0 / self.desired_fps());
            next_frame += period;
            let now = Instant::now();
            if next_frame > now {
                std::thread::sleep(next_frame - now);
            } else {
                next_frame = now;
            }
        }
    }

    fn timers(&mut self, dt: f32) {
        self.compositor_timer -= dt;
        if self.compositor_timer <= 0.0 {
            self.compositor_timer = 2.0;
            let ok = self.display.can_overlay();
            if ok != self.can_overlay {
                crate::log::debug(&format!("compositor {}", if ok { "started" } else { "stopped" }));
                self.can_overlay = ok;
                if let Some(t) = &self.tray {
                    let text = status_text(ok);
                    let _ = t.update(move |tray: &mut crate::tray::AppTray| tray.status = text);
                }
                self.build_screens();
            }
        }
        if let Some(t) = self.rebuild_in.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.rebuild_in = None;
                if self.display.monitors() != self.monitors {
                    crate::log::debug("monitors changed: rebuilding the overlays");
                    self.build_screens();
                }
            }
        }
        self.power_timer -= dt;
        if self.power_timer <= 0.0 {
            self.power_timer = 5.0;
            self.update_power_save();
        }
        if crate::updater::is_release_build() {
            self.update_timer -= dt;
            if self.update_timer <= 0.0 {
                self.update_timer = 6.0 * 3600.0;
                if self.cfg.auto_update {
                    self.check_for_update(false);
                }
            }
        }
        self.day_timer -= dt;
        if self.day_timer <= 0.0 {
            self.day_timer = 30.0;
            if crate::clock::today() != self.last_day {
                self.update_daily(false);
            }
        }
        if self.celebrate_at.is_some_and(|t| self.time >= t) {
            self.celebrate_at = None;
            if self.cfg.snowman {
                if let Some(s) = self.screens.iter().find(|s| s.layer.primary) {
                    if let Some(pos) = s.layer.snowman_pos() {
                        self.figures.snowman_celebrate(pos, s.layer.scale(), false);
                    }
                }
            }
        }
        // With snow on windows the scan runs 4 times a second, so piles follow moved windows and fall soon after a
        // minimize; every frame while a window with snow moves. Switched off, it only runs while snow is left.
        self.scan_timer -= dt;
        if self.scan_timer <= 0.0 {
            self.scan_timer = if !self.cfg.window_snow {
                1.0
            } else if self.window_snow.moving() {
                0.0
            } else {
                0.25
            };
            if self.cfg.window_snow || !self.window_snow.ledges.is_empty() {
                let rects: Vec<Rect> = self.screens.iter().map(|s| s.layer.rect).collect();
                let wins = crate::window_snow::scan(&self.display.client_windows(), &rects, self.display.dpi_scale);
                self.window_snow.update_ledges(wins);
            }
        }
        self.desktop_timer -= dt;
        if self.desktop_timer <= 0.0 {
            self.desktop_timer = 1.0;
            self.update_desktop();
        }
        self.top_timer -= dt;
        if self.top_timer <= 0.0 {
            self.top_timer = 2.0;
            let overlays: Vec<u32> = self.screens.iter().filter_map(|s| s.window.as_ref().map(|w| w.win)).collect();
            if !overlays.is_empty() && self.display.needs_raise(&overlays) {
                crate::log::debug("an app window came above the snow: raising the overlays");
                for s in &self.screens {
                    if let Some(w) = &s.window {
                        self.display.raise(w);
                    }
                }
            }
        }
    }

    /// One frame of the children's snowman on the primary screen: it is told where the plow is (which runs it
    /// down), sparkles fly when it is finished, and a few minutes later the plow is sent (`UpdateBuild` and the
    /// start of `UpdatePlow` on Windows).
    fn update_build(&mut self, dt: f32) {
        let Some(p) = self.screens.iter().position(|s| s.layer.primary) else { return };
        let layer = &self.screens[p].layer;
        // (a fullscreen app pauses the screen, and the story with it)
        if layer.paused() {
            return;
        }
        let (x0, s, width) = (layer.rect.x as f32, layer.scale(), layer.rect.w as f32);
        let pos = layer.snowman_pos();
        let plow = &self.figures.plow;
        let truck = (plow.active && plow.members.contains(&p)).then(|| ((plow.gx - x0) / s, plow.dir));
        let pushed = self.build.update(dt, width / s, truck);
        self.figures.plow.pushed += pushed * s;
        if self.build.just_finished {
            if let Some(pos) = pos {
                self.figures.snowman_celebrate(pos, s, true);
            }
        }
        if self.build.wants_plow {
            self.build.wants_plow = false;
            if self.cfg.plow && !self.figures.plow.active {
                let screens = self.screen_infos();
                self.figures.send_plow(&screens);
            }
        }
    }

    /// What the screens hand back after a frame: snow the plow scraped up, elves walking over to a neighbouring
    /// screen, and where an elf vanished by magic.
    fn after_frame(&mut self) {
        for i in 0..self.screens.len() {
            self.figures.plow.pushed += self.screens[i].layer.take_pushed();
            for (li, x, r) in self.screens[i].layer.take_ledge_hits() {
                if let Some(l) = self.window_snow.ledges.get_mut(li) {
                    l.deposit(x, r);
                }
            }
            for ci in self.screens[i].layer.take_clumps_landed() {
                if let Some(c) = self.window_snow.clumps.get_mut(ci) {
                    c.dead = true;
                }
            }
            let from_scale = self.screens[i].layer.scale();
            for (x, y, sc) in self.screens[i].layer.take_magic_puffs() {
                self.figures.magic_puff((x, y), sc, from_scale);
            }
            for elf in self.screens[i].layer.take_outgoing_elves() {
                let rects: Vec<Rect> = self.screens.iter().map(|s| s.layer.rect).collect();
                if let Some(j) = crate::figures::neighbor(&rects, i, elf.dir()) {
                    // its place goes with it (not for one that was sent away: that one is on its way out)
                    if elf.crossing() {
                        self.screens[i].layer.elf_balance -= 1;
                        self.screens[j].layer.elf_balance += 1;
                    }
                    self.screens[j].layer.receive_elf(elf, from_scale);
                }
            }
        }
        // Which screens can take one more elf as a guest in the next frame: at most one at a time, those already
        // on their way over counted.
        let rects: Vec<Rect> = self.screens.iter().map(|s| s.layer.rect).collect();
        let sides: Vec<[Option<usize>; 2]> = (0..rects.len())
            .map(|i| [crate::figures::neighbor(&rects, i, -1.0), crate::figures::neighbor(&rects, i, 1.0)])
            .collect();
        let room: Vec<bool> = (0..rects.len())
            .map(|i| {
                let from_left = sides[i][0].map_or(0, |j| self.screens[j].layer.elves_crossing(1.0));
                let from_right = sides[i][1].map_or(0, |j| self.screens[j].layer.elves_crossing(-1.0));
                self.screens[i].layer.elf_balance + from_left + from_right < 1
            })
            .collect();
        for (i, s) in self.screens.iter_mut().enumerate() {
            s.layer.guest_room_left = sides[i][0].is_some_and(|j| room[j]);
            s.layer.guest_room_right = sides[i][1].is_some_and(|j| room[j]);
        }
    }

    fn write_snapshots(&self, dir: &std::path::Path) {
        if let Err(e) = std::fs::create_dir_all(dir) {
            crate::log::error(&format!("snapshot dir {}: {e}", dir.display()));
            return;
        }
        for (i, s) in self.screens.iter().enumerate() {
            let path = dir.join(format!("screen{i}.png"));
            if let Err(e) = s.layer.write_snapshot(&path) {
                crate::log::error(&format!("snapshot {}: {e}", path.display()));
            }
        }
        crate::log::debug(&format!("snapshot written to {}", dir.display()));
        println!("snapshot written to {}", dir.display());
    }

    fn shutdown(&mut self) {
        self.sounds.hub.stop_all();
        if let Some(c) = self.calendar.take() {
            c.close(&self.display);
        }
        for s in self.screens.drain(..) {
            if let Some(w) = &s.window {
                self.display.destroy_overlay(w);
            }
        }
        if let Some(t) = self.tray.take() {
            let _ = t.shutdown();
        }
    }
}

/// Draws one frame on every screen and sends what changed. Per screen, so a failing screen (e.g. mid display
/// change) does not stop the others.
fn frame_all(display: &mut Display, screens: &mut [Screen], dt: f32, st: &SceneState) {
    // The X server reads shared memory when it gets to the request: let it finish last frame's first.
    if let Err(e) = display.sync() {
        crate::log::error(&format!("sync: {e}"));
    }
    for s in screens.iter_mut() {
        let layer = &mut s.layer;
        let present = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layer.frame(dt, st))) {
            Ok(p) => p,
            Err(_) => {
                crate::log::error("a screen's frame panicked");
                continue;
            }
        };
        let Some(w) = &s.window else { continue };
        // a fullscreen app pauses this screen: hide the window, and show it again afterwards
        if s.layer.paused() == s.mapped {
            s.mapped = !s.layer.paused();
            display.set_mapped(w, s.mapped);
            crate::log::debug(&format!(
                "screen at {},{} {}",
                s.layer.rect.x,
                s.layer.rect.y,
                if s.mapped { "resumed" } else { "paused for a fullscreen app" }
            ));
        }
        let width = s.layer.width();
        let result = match present {
            Present::Nothing => Ok(()),
            Present::Full => {
                let full = [Rect::new(0, 0, s.layer.rect.w, s.layer.rect.h)];
                display.put(w, s.layer.pixels(), width as usize, s.layer.height(), &full)
            }
            Present::Rects => {
                display.put(w, s.layer.pixels(), width as usize, s.layer.height(), s.layer.present_rects())
            }
        };
        if let Err(e) = result {
            crate::log::error(&format!("present: {e}"));
        }
    }
}

impl Sounds {
    fn new() -> Sounds {
        let mut hub = AudioHub::new();
        let jingles = (0..crate::sound::tunes().len())
            .map(|i| hub.create_voice(move || crate::sound::jingle(&crate::sound::tunes()[i]), true))
            .collect();
        let plow_engine = hub.create_voice(crate::sound::plow_engine, true);
        let plow_scrape = hub.create_voice(crate::sound::plow_scrape, true);
        let plow_horn = hub.create_voice(crate::sound::plow_horn, false);
        let geese = (0..crate::sound::GOOSE_MOODS.len())
            .map(|mood| hub.create_voice(move || crate::sound::geese(mood), true))
            .collect();
        let leaf_suction = hub.create_voice(crate::sound::leaf_suction, true);
        let butterfly_tune = hub.create_voice(crate::sound::muertos_son, true);
        let cart_roll =
            hub.create_voice(|| crate::sound::donkey_cart(crate::figures::donkey_step_cycle()), true);
        let broom_sweep =
            hub.create_voice(|| crate::sound::broom_sweep(crate::figures::donkey_step_cycle()), true);
        let donkey_bray = hub.create_voice(crate::sound::donkey_bray, false);
        let witch_tune = hub.create_voice(crate::sound::witch_tune, true);
        let ghost_wind = hub.create_voice(crate::sound::ghost_wind, true);
        let auld_tune = hub.create_voice(crate::sound::auld_lang_syne, true);
        let midnight_bell = hub.create_voice(crate::sound::midnight_bell, false);
        let drum_tune = hub.create_voice(crate::sound::batucada, true);
        let march_tune = hub.create_voice(crate::sound::rhein_band, true);
        Sounds {
            hub,
            drum_tune,
            march_tune,
            witch_tune,
            ghost_wind,
            auld_tune,
            midnight_bell,
            butterfly_tune,
            cart_roll,
            broom_sweep,
            donkey_bray,
            geese,
            leaf_suction,
            jingles,
            plow_engine,
            plow_scrape,
            plow_horn,
            jingle: 0,
            horned_this_run: false,
            santa_last: (0.0, 1.0, 0.0, 1.0),
        }
    }
}

/// The beat of carnival's drums, in seconds (120 to the minute).
const BEAT: f64 = 0.5;

/// One step of the ofrenda (De dødes dag) takes this long: seconds the app runs with the theme on.
const OFRENDA_STEP_TIME: f64 = 3600.0;

/// `JULE_SNOWMAN_SPEED=N` builds the ofrenda N times faster too (3600 gives a step a second).
fn ofrenda_speed() -> f64 {
    static SPEED: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SPEED.get_or_init(|| {
        std::env::var("JULE_SNOWMAN_SPEED")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .map_or(1.0, |v| v.max(0.01))
    })
}

/// The snowman gets one more step each day from the first start, up to the finished snowman.
fn snowman_stage_for(cfg: &Settings, today: NaiveDate) -> i32 {
    let days = cfg.first_run.map_or(0, |f| (today - f).num_days());
    (days + 1).clamp(1, SNOWMAN_MAX_STAGE as i64) as i32
}

/// "★ 12 dage til juleaften ★" in the menu.
fn countdown_text(today: NaiveDate) -> String {
    let eve = |y| NaiveDate::from_ymd_opt(y, 12, 24).unwrap_or(today);
    let mut e = eve(today.year());
    if today > e {
        e = eve(today.year() + 1);
    }
    text::countdown((e - today).num_days())
}

/// The window an event is about, for routing it to the calendar.
fn event_window(e: &Event) -> Option<u32> {
    Some(match e {
        Event::Expose(x) => x.window,
        Event::ButtonPress(x) => x.event,
        Event::MotionNotify(x) => x.event,
        Event::KeyPress(x) => x.event,
        Event::ClientMessage(x) => x.window,
        Event::DestroyNotify(x) => x.window,
        Event::ConfigureNotify(x) => x.window,
        Event::MapNotify(x) => x.window,
        Event::UnmapNotify(x) => x.window,
        _ => return None,
    })
}

fn status_text(can_overlay: bool) -> String {
    if can_overlay {
        String::new()
    } else {
        crate::text::NEEDS_COMPOSITOR.into()
    }
}

/// With JULE_DEBUG=1: the real frame rate, the time spent per frame, and the process's CPU use, every 5 seconds.
struct Stats {
    start: Instant,
    frames: u32,
    busy: Duration,
    cpu_start: Option<f64>,
}

impl Stats {
    fn new() -> Self {
        Stats { start: Instant::now(), frames: 0, busy: Duration::ZERO, cpu_start: cpu_seconds() }
    }

    fn frame(&mut self, busy: Duration, wanted: f64, flakes: usize) {
        self.frames += 1;
        self.busy += busy;
        let wall = self.start.elapsed().as_secs_f64();
        if wall < 5.0 {
            return;
        }
        if crate::log::debug_enabled() {
            let cpu = cpu_seconds();
            let cpu_pct = match (self.cpu_start, cpu) {
                (Some(a), Some(b)) => format!("{:.1}%", (b - a) / wall * 100.0),
                _ => "?".into(),
            };
            crate::log::debug(&format!(
                "fps {:.1} (wanted {wanted}), frame {:.2} ms, cpu {cpu_pct}, flakes {flakes}",
                self.frames as f64 / wall,
                self.busy.as_secs_f64() * 1000.0 / self.frames.max(1) as f64
            ));
        }
        *self = Stats::new();
    }
}

/// User + system CPU seconds of this process, from /proc/self/stat (clock ticks of 1/100 s, the Linux default).
fn cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // fields after the command name (which may contain spaces) start at field 3
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let utime: f64 = f.get(11)?.parse().ok()?;
    let stime: f64 = f.get(12)?.parse().ok()?;
    Some((utime + stime) / 100.0)
}
